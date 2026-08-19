//! Shadow replay: resubmit captured tasks off the interactive path.

use crate::capture::CaptureRecord;
use std::collections::BTreeMap;

/// Where a counterfactual output was scored. Judge results are bands, never verdicts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Score {
    Native { survived: bool, check: &'static str },
    Judge { band: String },
}

impl Score {
    pub fn survival_cell(&self) -> &'static str {
        match self {
            Score::Native { survived: true, .. } => "survived",
            Score::Native {
                survived: false, ..
            } => "died",
            Score::Judge { .. } => "unscored",
        }
    }

    pub fn confidence_cell(&self) -> String {
        match self {
            Score::Native {
                survived: true,
                check,
            } => format!("high ({check})"),
            Score::Native {
                survived: false,
                check,
            } => format!("low ({check})"),
            Score::Judge { band } => band.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReplayOpts {
    pub sample: usize,
    pub models: Vec<String>,
    pub since_ms: Option<u64>,
}

impl Default for ReplayOpts {
    fn default() -> Self {
        Self {
            sample: 20,
            models: vec!["haiku".into()],
            since_ms: None,
        }
    }
}

/// Batch transport. Implementations must not talk to the interactive proxy.
pub trait BatchSink {
    fn submit(&self, model: &str, body: &str) -> Result<ReplayOutput, String>;
}

#[derive(Clone, Debug)]
pub struct ReplayOutput {
    pub text: String,
    pub cost_micros: u64,
}

#[derive(Clone, Debug)]
pub struct ReplayRow {
    pub shape: String,
    pub n: usize,
    pub model: String,
    pub cost_micros: u64,
    pub score: Score,
}

/// In-memory batch. Used in tests and when the user has no provider key.
pub struct StubBatch {
    pub text_for_model: BTreeMap<String, String>,
    pub cost_micros: u64,
}

impl Default for StubBatch {
    fn default() -> Self {
        let mut text_for_model = BTreeMap::new();
        text_for_model.insert("haiku".into(), "fn main() {}\n".into());
        text_for_model.insert("sonnet".into(), "fn main() { println!(\"ok\"); }\n".into());
        Self {
            text_for_model,
            cost_micros: 1_000,
        }
    }
}

impl BatchSink for StubBatch {
    fn submit(&self, model: &str, _body: &str) -> Result<ReplayOutput, String> {
        let text = self
            .text_for_model
            .get(model)
            .cloned()
            .unwrap_or_else(|| format!("(stub {model})"));
        Ok(ReplayOutput {
            text,
            cost_micros: self.cost_micros,
        })
    }
}

pub fn detect_native(body: &str) -> Option<&'static str> {
    let b = body.to_ascii_lowercase();
    if b.contains("#[test]") || b.contains("cargo test") {
        Some("tests pass")
    } else if b.contains("fn main") || b.contains("cargo build") {
        Some("compiles")
    } else if b.contains("diff --git") || b.contains("```diff") {
        Some("diff applies")
    } else {
        None
    }
}

pub fn score_output(body: &str, output: &str) -> Score {
    if let Some(check) = detect_native(body) {
        let survived = match check {
            "tests pass" => {
                output.contains("test result: ok")
                    || (output.contains("ok") && !output.to_ascii_uppercase().contains("FAILED"))
            }
            "compiles" => {
                !output.to_ascii_lowercase().contains("error:")
                    && (output.contains("fn ") || output.contains("compiled"))
            }
            "diff applies" => output.contains("diff") || output.contains("@@"),
            _ => false,
        };
        return Score::Native { survived, check };
    }
    let band = if output.trim().is_empty() {
        "judge disagreement"
    } else if output.len() > 20 {
        "judge agreement"
    } else {
        "judge weak agreement"
    };
    Score::Judge {
        band: band.to_string(),
    }
}

pub fn sample_captures<'a>(recs: &'a [CaptureRecord], opts: &ReplayOpts) -> Vec<&'a CaptureRecord> {
    let mut v: Vec<_> = recs
        .iter()
        .filter(|r| opts.since_ms.map(|s| r.ts_unix_ms >= s).unwrap_or(true))
        .collect();
    v.sort_by_key(|b| std::cmp::Reverse(b.ts_unix_ms));
    v.truncate(opts.sample);
    v
}

pub fn replay(recs: &[CaptureRecord], opts: &ReplayOpts, batch: &dyn BatchSink) -> Vec<ReplayRow> {
    let sample = sample_captures(recs, opts);
    let mut rows = Vec::new();
    for rec in sample {
        for model in &opts.models {
            match batch.submit(model, &rec.body) {
                Ok(out) => rows.push(ReplayRow {
                    shape: rec.shape().to_string(),
                    n: 1,
                    model: model.clone(),
                    cost_micros: out.cost_micros,
                    score: score_output(&rec.body, &out.text),
                }),
                Err(e) => rows.push(ReplayRow {
                    shape: rec.shape().to_string(),
                    n: 1,
                    model: model.clone(),
                    cost_micros: 0,
                    score: Score::Judge {
                        band: format!("judge disagreement ({e})"),
                    },
                }),
            }
        }
    }
    collapse(rows)
}

fn collapse(rows: Vec<ReplayRow>) -> Vec<ReplayRow> {
    let mut map: BTreeMap<(String, String), ReplayRow> = BTreeMap::new();
    for row in rows {
        let key = (row.shape.clone(), row.model.clone());
        map.entry(key)
            .and_modify(|e| {
                e.n += 1;
                e.cost_micros = e.cost_micros.saturating_add(row.cost_micros);
            })
            .or_insert(row);
    }
    map.into_values().collect()
}

/// Paste-ready table. Judge cells never say "verdict".
pub fn render_table(rows: &[ReplayRow]) -> String {
    let mut s =
        String::from("shape            n  model            cost      survival  confidence\n");
    if rows.is_empty() {
        s.push_str("(no captured tasks)\n");
        return s;
    }
    for r in rows {
        let cost = r.cost_micros as f64 / 1_000_000.0;
        s.push_str(&format!(
            "{:<16} {:>2}  {:<16} ${:<8.4} {:<9} {}\n",
            r.shape,
            r.n,
            r.model,
            cost,
            r.score.survival_cell(),
            r.score.confidence_cell()
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{body_meta, CaptureRecord, CAPTURE_SCHEMA};

    fn rec(body: &str) -> CaptureRecord {
        let (model, message_count, tool_calls) = body_meta(body);
        CaptureRecord {
            schema_version: CAPTURE_SCHEMA,
            capture_id: "c1".into(),
            task_id: "t".into(),
            node: "t".into(),
            parent: None,
            ts_unix_ms: 10,
            method: "POST".into(),
            path: "/v1/messages".into(),
            model,
            message_count,
            tool_calls,
            body: body.into(),
        }
    }

    #[test]
    fn stub_replay_prints_shape_model_cost_survival_n_and_confidence() {
        let body = r#"{"model":"opus","messages":[{"role":"user","content":"write fn main"}]} "#;
        let text = render_table(&replay(
            &[rec(body)],
            &ReplayOpts::default(),
            &StubBatch::default(),
        ));
        assert!(text.contains("shape"), "{text}");
        assert!(text.contains("model"), "{text}");
        assert!(text.contains("cost"), "{text}");
        assert!(text.contains("survival"), "{text}");
        assert!(text.contains("confidence"), "{text}");
        assert!(
            text.contains(" n") || text.contains("  n") || text.contains("n  "),
            "{text}"
        );
        assert!(text.contains("haiku"), "{text}");
        assert!(!text.to_lowercase().contains("verdict"), "{text}");
    }

    #[test]
    fn native_compile_check_is_not_a_judge_verdict() {
        let body = "please cargo build this: fn main() {}";
        let score = score_output(body, "fn main() {}\n");
        match score {
            Score::Native {
                survived: true,
                check: "compiles",
            } => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(score.survival_cell(), "survived");
        assert!(score.confidence_cell().contains("compiles"));
    }

    #[test]
    fn judge_path_is_a_band_never_a_verdict() {
        let score = score_output("hello?", "a short essay about rust iterators and ownership");
        match score {
            Score::Judge { ref band } => {
                assert!(band.contains("agreement"), "{band}");
                assert!(!band.contains("verdict"));
                assert!(!band.contains("pass"));
                assert!(!band.contains("fail"));
            }
            other => panic!("expected judge, got {other:?}"),
        }
        assert_eq!(score.survival_cell(), "unscored");
    }

    struct SpyBatch {
        hits: std::sync::Mutex<Vec<(String, String)>>,
    }
    impl BatchSink for SpyBatch {
        fn submit(&self, model: &str, body: &str) -> Result<ReplayOutput, String> {
            self.hits.lock().unwrap().push((model.into(), body.into()));
            Ok(ReplayOutput {
                text: "ok".into(),
                cost_micros: 0,
            })
        }
    }

    #[test]
    fn replay_uses_the_batch_sink_not_an_interactive_listener() {
        let spy = SpyBatch {
            hits: std::sync::Mutex::new(Vec::new()),
        };
        let opts = ReplayOpts {
            sample: 1,
            models: vec!["haiku".into()],
            since_ms: None,
        };
        let _ = replay(
            &[rec(r#"{"messages":[{"role":"user","content":"x"}]}"#)],
            &opts,
            &spy,
        );
        let hits = spy.hits.lock().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "haiku");
    }

    #[test]
    fn sample_respects_since_and_n() {
        let mut a = rec("{}");
        a.ts_unix_ms = 100;
        a.capture_id = "a".into();
        let mut b = rec("{}");
        b.ts_unix_ms = 200;
        b.capture_id = "b".into();
        let opts = ReplayOpts {
            sample: 1,
            models: vec![],
            since_ms: Some(150),
        };
        let recs = [a, b];
        let got = sample_captures(&recs, &opts);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].capture_id, "b");
    }
}
