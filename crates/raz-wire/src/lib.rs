//! Incremental SSE decoding and provider-agnostic usage extraction.
//!
//! # The passthrough discipline
//!
//! This crate never owns the response body. It is fed bytes that are *already
//! on their way to the client*, and it extracts what it needs without holding,
//! reordering, or reserializing anything.
//!
//! That constraint is not stylistic. Anthropic's gateway guidance is explicit:
//! a gateway that wraps or rewrites upstream payloads breaks the client's error
//! recovery path and the header/body pairing that beta features depend on, and
//! Claude Code aborts a stream that goes silent for 300 seconds. A proxy that
//! buffers to parse is a proxy that stalls.
//!
//! So: bytes flow through, frames are cloned to the meter, and the meter is
//! allowed to fail without affecting the relay.

#![forbid(unsafe_code)]

use serde_json::Value;

/// One decoded `text/event-stream` frame.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

/// Feed bytes in, get frames out. Handles arbitrary chunk boundaries — a frame
/// split across three TCP segments decodes exactly once.
#[derive(Default)]
pub struct SseDecoder {
    buf: String,
    event: Option<String>,
    data: Vec<String>,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a chunk. Returns whatever frames completed.
    ///
    /// Invalid UTF-8 is dropped rather than erroring: metering must never be a
    /// reason the user's request fails.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        self.buf.push_str(&String::from_utf8_lossy(chunk));
        let mut out = Vec::new();

        while let Some(nl) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=nl).collect();
            let line = line.trim_end_matches(['\n', '\r']);

            if line.is_empty() {
                if !self.data.is_empty() || self.event.is_some() {
                    out.push(SseFrame {
                        event: self.event.take(),
                        data: std::mem::take(&mut self.data).join("\n"),
                    });
                }
                continue;
            }
            if let Some(v) = line.strip_prefix("event:") {
                self.event = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("data:") {
                self.data.push(v.strip_prefix(' ').unwrap_or(v).to_string());
            }
            // Comments (`:`) and unknown fields are ignored, per the SSE spec.
        }
        out
    }
}

/// Token counts, normalized across providers.
///
/// Cache writes and reads are kept apart because they are priced apart —
/// reads at 0.1x input, 5-minute writes at 1.25x, 1-hour writes at 2x. Summing
/// them is how tools end up reporting savings while the bill goes up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub reasoning: u64,
}

impl Usage {
    /// Total tokens moved. Useful for volume, useless for cost.
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }

    /// Cost in micro-dollars, given a rate card. Integer math on purpose: a
    /// ledger that accumulates f64 drifts, and this one is used to enforce a
    /// ceiling.
    pub fn micros(&self, r: &RateCard) -> u64 {
        (self.input * r.input
            + self.output * r.output
            + self.cache_write_5m * r.cache_write_5m
            + self.cache_write_1h * r.cache_write_1h
            + self.cache_read * r.cache_read)
            / 1_000_000
    }

    pub fn merge(&mut self, other: &Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
        self.cache_read += other.cache_read;
        self.reasoning += other.reasoning;
    }

    /// Ratio of cache reads to writes. Below ~1 means the prefix keeps changing
    /// and you are re-paying for context at up to 12.5x the read price — the
    /// signature of a compression or routing layer fighting the cache.
    pub fn cache_read_write_ratio(&self) -> Option<f64> {
        let w = self.cache_write_5m + self.cache_write_1h;
        (w > 0).then(|| self.cache_read as f64 / w as f64)
    }
}

/// Micro-dollars per million tokens. Loaded from config; never hardcoded, and
/// never assumed to match a negotiated contract.
#[derive(Clone, Copy, Debug)]
pub struct RateCard {
    pub input: u64,
    pub output: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
}

impl RateCard {
    /// Derive the cache multipliers from a base input rate, per Anthropic's
    /// published ratios. A convenience for config files, not a price list.
    pub fn from_base(input: u64, output: u64) -> Self {
        Self {
            input,
            output,
            cache_write_5m: input * 5 / 4,
            cache_write_1h: input * 2,
            cache_read: input / 10,
        }
    }
}

/// A provider's wire dialect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// `/v1/messages` — usage arrives split across `message_start` and `message_delta`.
    AnthropicMessages,
    /// `/v1/chat/completions` — usage arrives in one final chunk, and only when
    /// the caller set `stream_options.include_usage`.
    OpenAiChat,
}

impl Dialect {
    pub fn for_path(path: &str) -> Option<Self> {
        match path {
            p if p.ends_with("/v1/messages") => Some(Self::AnthropicMessages),
            p if p.ends_with("/chat/completions") => Some(Self::OpenAiChat),
            _ => None,
        }
    }
}

/// Accumulates usage across a stream.
pub struct Meter {
    dialect: Dialect,
    usage: Usage,
    saw_terminal: bool,
}

impl Meter {
    pub fn new(dialect: Dialect) -> Self {
        Self { dialect, usage: Usage::default(), saw_terminal: false }
    }

    /// Observe one frame. Never fails; unparseable frames are skipped.
    pub fn observe(&mut self, frame: &SseFrame) {
        if frame.data == "[DONE]" {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(&frame.data) else { return };
        match self.dialect {
            Dialect::AnthropicMessages => self.observe_anthropic(frame, &v),
            Dialect::OpenAiChat => self.observe_openai(&v),
        }
    }

    fn observe_anthropic(&mut self, frame: &SseFrame, v: &Value) {
        let kind = frame.event.as_deref().or_else(|| v.get("type")?.as_str());
        match kind {
            // Prompt-side counts land here, nested under `message`.
            Some("message_start") => {
                if let Some(u) = v.pointer("/message/usage") {
                    self.take_anthropic(u, false);
                }
            }
            // Final output count lands here. This is the terminal signal.
            Some("message_delta") => {
                if let Some(u) = v.get("usage") {
                    self.take_anthropic(u, true);
                    self.saw_terminal = true;
                }
            }
            _ => {}
        }
    }

    fn take_anthropic(&mut self, u: &Value, delta: bool) {
        let g = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        if !delta {
            self.usage.input += g("input_tokens");
            self.usage.cache_read += g("cache_read_input_tokens");
            // Prefer the split breakdown; fall back to the flat total.
            let m5 = u.pointer("/cache_creation/ephemeral_5m_input_tokens").and_then(Value::as_u64);
            let m1 = u.pointer("/cache_creation/ephemeral_1h_input_tokens").and_then(Value::as_u64);
            match (m5, m1) {
                (None, None) => self.usage.cache_write_5m += g("cache_creation_input_tokens"),
                _ => {
                    self.usage.cache_write_5m += m5.unwrap_or(0);
                    self.usage.cache_write_1h += m1.unwrap_or(0);
                }
            }
        }
        // `message_delta` reports cumulative output, so assign rather than add.
        let out = g("output_tokens");
        if out > 0 {
            self.usage.output = out;
        }
        if let Some(t) = u.pointer("/output_tokens_details/thinking_tokens").and_then(Value::as_u64)
        {
            self.usage.reasoning = t;
        }
    }

    fn observe_openai(&mut self, v: &Value) {
        let Some(u) = v.get("usage").filter(|u| !u.is_null()) else { return };
        let g = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        let cached = u
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.usage.input = g("prompt_tokens").saturating_sub(cached);
        self.usage.cache_read = cached;
        self.usage.output = g("completion_tokens");
        self.usage.reasoning = u
            .pointer("/completion_tokens_details/reasoning_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.saw_terminal = true;
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }

    /// Whether the stream delivered its terminal usage frame.
    ///
    /// False means the client aborted, the upstream died, or — for OpenAI —
    /// the caller never set `stream_options.include_usage`. A ledger that
    /// silently records zero for these under-reports real spend, so callers
    /// must decide explicitly what to do.
    pub fn is_complete(&self) -> bool {
        self.saw_terminal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(d: &mut SseDecoder, s: &str) -> Vec<SseFrame> {
        d.push(s.as_bytes())
    }

    #[test]
    fn decodes_a_simple_frame() {
        let mut d = SseDecoder::new();
        let f = feed(&mut d, "event: ping\ndata: {\"a\":1}\n\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].event.as_deref(), Some("ping"));
        assert_eq!(f[0].data, "{\"a\":1}");
    }

    #[test]
    fn a_frame_split_across_chunks_decodes_once() {
        let mut d = SseDecoder::new();
        assert!(feed(&mut d, "event: mes").is_empty());
        assert!(feed(&mut d, "sage_delta\ndata: {\"x\"").is_empty());
        let f = feed(&mut d, ":2}\n\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].event.as_deref(), Some("message_delta"));
        assert_eq!(f[0].data, "{\"x\":2}");
    }

    #[test]
    fn handles_crlf_and_multiline_data() {
        let mut d = SseDecoder::new();
        let f = feed(&mut d, "data: one\r\ndata: two\r\n\r\n");
        assert_eq!(f[0].data, "one\ntwo");
    }

    #[test]
    fn ignores_comments_and_unknown_fields() {
        let mut d = SseDecoder::new();
        let f = feed(&mut d, ": keepalive\nid: 7\ndata: x\n\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].data, "x");
    }

    #[test]
    fn invalid_utf8_does_not_panic() {
        let mut d = SseDecoder::new();
        d.push(&[0xff, 0xfe]);
        d.push(b"data: ok\n\n");
        // Whatever it decoded, it survived.
        assert!(d.push(b"").is_empty());
    }

    #[test]
    fn anthropic_usage_splits_cache_tiers() {
        let mut m = Meter::new(Dialect::AnthropicMessages);
        let mut d = SseDecoder::new();
        let start = r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":12,"cache_read_input_tokens":6000,"cache_creation":{"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":900}}}}

"#;
        for f in feed(&mut d, start) {
            m.observe(&f);
        }
        let delta = r#"event: message_delta
data: {"type":"message_delta","usage":{"output_tokens":250,"output_tokens_details":{"thinking_tokens":100}}}

"#;
        for f in feed(&mut d, delta) {
            m.observe(&f);
        }
        let u = m.usage();
        assert_eq!(u.input, 12);
        assert_eq!(u.cache_read, 6000);
        assert_eq!(u.cache_write_5m, 100);
        assert_eq!(u.cache_write_1h, 900);
        assert_eq!(u.output, 250);
        assert_eq!(u.reasoning, 100);
        assert!(m.is_complete());
    }

    #[test]
    fn anthropic_flat_cache_field_still_counts() {
        let mut m = Meter::new(Dialect::AnthropicMessages);
        let mut d = SseDecoder::new();
        let s = "event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":1,\"cache_creation_input_tokens\":500}}}\n\n";
        for f in feed(&mut d, s) {
            m.observe(&f);
        }
        assert_eq!(m.usage().cache_write_5m, 500);
    }

    #[test]
    fn anthropic_output_is_cumulative_not_additive() {
        let mut m = Meter::new(Dialect::AnthropicMessages);
        let mut d = SseDecoder::new();
        for chunk in [
            "event: message_delta\ndata: {\"usage\":{\"output_tokens\":100}}\n\n",
            "event: message_delta\ndata: {\"usage\":{\"output_tokens\":250}}\n\n",
        ] {
            for f in feed(&mut d, chunk) {
                m.observe(&f);
            }
        }
        assert_eq!(m.usage().output, 250, "must not double-count to 350");
    }

    #[test]
    fn an_aborted_stream_is_marked_incomplete() {
        let mut m = Meter::new(Dialect::AnthropicMessages);
        let mut d = SseDecoder::new();
        let s = "event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":9}}}\n\n";
        for f in feed(&mut d, s) {
            m.observe(&f);
        }
        assert!(!m.is_complete(), "no message_delta means we never saw the total");
        assert_eq!(m.usage().input, 9, "but what we did see is still recorded");
    }

    #[test]
    fn openai_usage_separates_cached_prompt_tokens() {
        let mut m = Meter::new(Dialect::OpenAiChat);
        let mut d = SseDecoder::new();
        let s = r#"data: {"usage":{"prompt_tokens":1000,"completion_tokens":40,"prompt_tokens_details":{"cached_tokens":800},"completion_tokens_details":{"reasoning_tokens":15}}}

"#;
        for f in feed(&mut d, s) {
            m.observe(&f);
        }
        let u = m.usage();
        assert_eq!(u.input, 200, "prompt_tokens includes cached; they must be split out");
        assert_eq!(u.cache_read, 800);
        assert_eq!(u.output, 40);
        assert_eq!(u.reasoning, 15);
        assert!(m.is_complete());
    }

    #[test]
    fn openai_without_include_usage_is_incomplete_not_zero() {
        let mut m = Meter::new(Dialect::OpenAiChat);
        let mut d = SseDecoder::new();
        for f in feed(&mut d, "data: {\"choices\":[{\"delta\":{}}]}\n\ndata: [DONE]\n\n") {
            m.observe(&f);
        }
        assert!(!m.is_complete());
    }

    #[test]
    fn garbage_frames_are_skipped() {
        let mut m = Meter::new(Dialect::OpenAiChat);
        m.observe(&SseFrame { event: None, data: "not json".into() });
        m.observe(&SseFrame { event: None, data: "{".into() });
        assert_eq!(m.usage(), Usage::default());
    }

    #[test]
    fn cost_respects_the_cache_multipliers() {
        // $15/M in, $75/M out => reads 1.5, 5m writes 18.75, 1h writes 30.
        let r = RateCard::from_base(15_000_000, 75_000_000);
        assert_eq!(r.cache_read, 1_500_000);
        assert_eq!(r.cache_write_5m, 18_750_000);
        assert_eq!(r.cache_write_1h, 30_000_000);

        let u = Usage { cache_read: 1_000_000, ..Default::default() };
        let v = Usage { cache_write_1h: 1_000_000, ..Default::default() };
        assert_eq!(u.micros(&r), 1_500_000);
        assert_eq!(v.micros(&r), 30_000_000);
        assert_eq!(v.micros(&r) / u.micros(&r), 20, "a 1h write costs 20x a read");
    }

    #[test]
    fn cache_ratio_flags_a_thrashing_prefix() {
        let healthy = Usage { cache_read: 8_000, cache_write_5m: 1_000, ..Default::default() };
        let thrashing = Usage { cache_read: 500, cache_write_5m: 5_000, ..Default::default() };
        assert!(healthy.cache_read_write_ratio().unwrap() > 5.0);
        assert!(thrashing.cache_read_write_ratio().unwrap() < 1.0);
        assert_eq!(Usage::default().cache_read_write_ratio(), None);
    }

    #[test]
    fn dialect_is_chosen_by_path() {
        assert_eq!(Dialect::for_path("/v1/messages"), Some(Dialect::AnthropicMessages));
        assert_eq!(Dialect::for_path("/v1/chat/completions"), Some(Dialect::OpenAiChat));
        assert_eq!(Dialect::for_path("/healthz"), None);
    }
}
