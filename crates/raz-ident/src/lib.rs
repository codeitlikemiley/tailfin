//! Task identity inference from LLM wire traffic.
//!
//! Every product that enforces a per-task budget today requires the *caller* to
//! declare the task boundary — an SDK context manager, a registered agent, a
//! trace-id header, a hosted harness. None can draw a boundary around an
//! unmodified coding agent that just opens an HTTPS connection.
//!
//! This crate draws it anyway, from two signals:
//!
//! 1. **Declared identity** — headers some agents already emit. Anthropic
//!    publishes `x-claude-code-session-id` / `-agent-id` / `-parent-agent-id`
//!    *specifically* so a gateway can attribute cost to parallel agents.
//! 2. **Inferred identity** — for everything else, a rolling prefix digest over
//!    the message array.
//!
//! The second one is the interesting half. The signal that tells you two
//! requests belong to the same conversation is *the same signal* the provider
//! uses to decide whether its prompt cache hits: a stable, shared, ordered
//! prefix. So prefix-matching is not a heuristic bolted on the side — it is the
//! wire's own notion of continuity.

#![forbid(unsafe_code)]

use std::collections::HashMap;

/// Prefix depths we digest at. Powers of two so a session that grows past a
/// level keeps every shallower level intact.
pub const LEVELS: [usize; 6] = [1, 2, 4, 8, 16, 32];

/// Cumulative FNV-1a hashes of a conversation prefix at each depth in [`LEVELS`].
///
/// A level is `None` when the conversation is shorter than that depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PrefixDigest {
    levels: [Option<u64>; LEVELS.len()],
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

impl PrefixDigest {
    /// Digest an ordered sequence of canonical message representations.
    ///
    /// `messages[i]` should be a stable, canonical byte encoding of message `i`
    /// — role plus content, with anything volatile (timestamps, request ids)
    /// already stripped. Garbage in, unstable sessions out.
    pub fn from_messages<'a, I, B>(messages: I) -> Self
    where
        I: IntoIterator<Item = &'a B>,
        B: AsRef<[u8]> + 'a,
    {
        let mut levels = [None; LEVELS.len()];
        let mut rolling = FNV_OFFSET;

        for (idx, msg) in messages.into_iter().enumerate() {
            rolling = fnv1a(rolling, msg.as_ref());
            // Separator so ["ab","c"] and ["a","bc"] differ.
            rolling = fnv1a(rolling, b"\x1e");
            let depth = idx + 1;
            for (li, lv) in LEVELS.iter().enumerate() {
                if depth == *lv {
                    levels[li] = Some(rolling);
                }
            }
            if depth >= *LEVELS.last().unwrap() {
                break;
            }
        }
        Self { levels }
    }

    /// Deepest level index at which two digests agree, if any.
    ///
    /// Returns the *index into [`LEVELS`]*, so a return of `Some(3)` means the
    /// first 8 messages are identical.
    pub fn match_level(&self, other: &Self) -> Option<usize> {
        let mut best = None;
        for i in 0..LEVELS.len() {
            match (self.levels[i], other.levels[i]) {
                (Some(a), Some(b)) if a == b => best = Some(i),
                // Once a level disagrees, no deeper level can agree.
                (Some(_), Some(_)) => break,
                _ => break,
            }
        }
        best
    }

    /// Number of leading messages known to be shared.
    pub fn shared_depth(&self, other: &Self) -> usize {
        self.match_level(other).map(|i| LEVELS[i]).unwrap_or(0)
    }

    /// True when the conversation is long enough to identify at level `idx`.
    pub fn has_level(&self, idx: usize) -> bool {
        self.levels.get(idx).copied().flatten().is_some()
    }
}

/// Where a request's identity came from. Carried through to the ledger so a
/// report can say how much of its own attribution it actually trusts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentitySource {
    /// Agent declared it. Attribution is exact.
    Declared {
        session: String,
        node: Option<String>,
        parent: Option<String>,
    },
    /// We inferred it from a shared prefix at `shared_depth` messages.
    Inferred { shared_depth: usize },
    /// No usable signal — a single-shot request or an agent we can't read.
    Anonymous,
}

impl IdentitySource {
    /// Rough confidence, for reports that need to disclose their own weakness.
    pub fn confidence(&self) -> f32 {
        match self {
            IdentitySource::Declared { .. } => 1.0,
            // Deeper shared prefix, more confidence. 32 messages ≈ certain.
            IdentitySource::Inferred { shared_depth } => {
                (*shared_depth as f32 / 32.0).clamp(0.25, 0.99)
            }
            IdentitySource::Anonymous => 0.0,
        }
    }
    pub fn is_declared(&self) -> bool {
        matches!(self, IdentitySource::Declared { .. })
    }
}

/// A resolved position in the task tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeRef {
    /// Root of the task tree. The budget unit.
    pub root: String,
    /// This node. Equals `root` for a main-thread request.
    pub node: String,
    /// Parent node, when this is a subagent.
    pub parent: Option<String>,
    pub source: IdentitySource,
}

impl NodeRef {
    pub fn is_root(&self) -> bool {
        self.node == self.root
    }
}

/// Minimal view of request headers, so this crate stays free of HTTP deps.
pub trait Headers {
    fn get(&self, name: &str) -> Option<&str>;
}

impl Headers for HashMap<String, String> {
    fn get(&self, name: &str) -> Option<&str> {
        HashMap::get(self, name).map(|s| s.as_str())
    }
}

/// Read declared identity from headers.
///
/// Handles the two agents that publish a task tree today. Everything else falls
/// through to prefix inference.
pub fn from_headers<H: Headers>(h: &H) -> Option<IdentitySource> {
    // Claude Code. `-agent-id` appears only on subagent requests;
    // `-parent-agent-id` only on nested ones.
    if let Some(session) = h.get("x-claude-code-session-id") {
        return Some(IdentitySource::Declared {
            session: session.to_string(),
            node: h.get("x-claude-code-agent-id").map(str::to_string),
            parent: h.get("x-claude-code-parent-agent-id").map(str::to_string),
        });
    }
    // Codex CLI ships a JSON blob. `root_turn_id` is the task-tree root — the
    // single best field for this purpose in the whole ecosystem.
    if let Some(raw) = h.get("x-codex-turn-metadata") {
        if let Some(m) = parse_codex_metadata(raw) {
            return Some(m);
        }
    }
    None
}

/// Extract the three fields we need from Codex's turn-metadata blob without
/// pulling in a JSON dependency: this runs on every request.
fn parse_codex_metadata(raw: &str) -> Option<IdentitySource> {
    let root = scrape_json_string(raw, "root_turn_id").or_else(|| scrape_json_string(raw, "session_id"))?;
    let turn = scrape_json_string(raw, "turn_id");
    let parent = scrape_json_string(raw, "parent_turn_id");
    Some(IdentitySource::Declared { session: root, node: turn, parent })
}

/// Pull `"key":"value"` out of flat JSON. Deliberately not a parser — it must
/// not panic, allocate much, or care about unknown shapes.
fn scrape_json_string(hay: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let start = hay.find(&pat)? + pat.len();
    let rest = &hay[start..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let mut chars = after.char_indices();
    if chars.next()?.1 != '"' {
        return None; // null or non-string
    }
    let mut out = String::new();
    let mut escaped = false;
    for (_, c) in chars {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

/// Tracks live conversations so undeclared requests can be attached to one.
#[derive(Default)]
pub struct SessionIndex {
    live: Vec<Live>,
    /// Minimum shared messages before we'll claim two requests are one session.
    /// Two is enough to clear a shared system prompt; one is not.
    min_level_idx: usize,
    next: u64,
}

struct Live {
    root: String,
    digest: PrefixDigest,
    /// Deepest digest seen for this session, so it grows as the conversation does.
    depth_seen: usize,
}

impl SessionIndex {
    pub fn new() -> Self {
        Self { live: Vec::new(), min_level_idx: 1, next: 0 }
    }

    /// Require a deeper shared prefix before joining. Raise this when many
    /// unrelated sessions share a long system prompt.
    pub fn with_min_level(mut self, idx: usize) -> Self {
        self.min_level_idx = idx.min(LEVELS.len() - 1);
        self
    }

    /// Resolve a request to a node, declared identity taking precedence.
    pub fn resolve<H: Headers>(&mut self, headers: &H, digest: Option<PrefixDigest>) -> NodeRef {
        if let Some(src @ IdentitySource::Declared { .. }) = from_headers(headers) {
            let (session, node, parent) = match &src {
                IdentitySource::Declared { session, node, parent } => {
                    (session.clone(), node.clone(), parent.clone())
                }
                _ => unreachable!(),
            };
            return NodeRef {
                node: node.clone().unwrap_or_else(|| session.clone()),
                // A nested agent's parent is its declared parent; a top-level
                // subagent's parent is the session root.
                parent: parent.or_else(|| node.as_ref().map(|_| session.clone())),
                root: session,
                source: src,
            };
        }

        let Some(d) = digest else {
            return self.anonymous();
        };

        let mut best: Option<(usize, usize)> = None; // (live idx, level idx)
        for (i, live) in self.live.iter().enumerate() {
            if let Some(lv) = live.digest.match_level(&d) {
                if lv >= self.min_level_idx && best.is_none_or(|(_, b)| lv > b) {
                    best = Some((i, lv));
                }
            }
        }

        match best {
            Some((i, lv)) => {
                let shared = LEVELS[lv];
                if shared > self.live[i].depth_seen {
                    self.live[i].depth_seen = shared;
                    self.live[i].digest = d;
                }
                let root = self.live[i].root.clone();
                NodeRef {
                    node: root.clone(),
                    parent: None,
                    root,
                    source: IdentitySource::Inferred { shared_depth: shared },
                }
            }
            None => {
                self.next += 1;
                let root = format!("inferred-{:06}", self.next);
                self.live.push(Live { root: root.clone(), digest: d, depth_seen: 0 });
                NodeRef {
                    node: root.clone(),
                    parent: None,
                    root,
                    source: IdentitySource::Inferred { shared_depth: 0 },
                }
            }
        }
    }

    fn anonymous(&mut self) -> NodeRef {
        self.next += 1;
        let root = format!("anon-{:06}", self.next);
        NodeRef { node: root.clone(), parent: None, root, source: IdentitySource::Anonymous }
    }

    pub fn live_sessions(&self) -> usize {
        self.live.len()
    }

    /// Drop sessions not seen recently. Called on a timer by the proxy; the
    /// index is bounded memory, not a database.
    pub fn retain<F: FnMut(&str) -> bool>(&mut self, mut keep: F) {
        self.live.retain(|l| keep(&l.root));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("msg-{i}")).collect()
    }

    #[test]
    fn identical_prefixes_match_at_full_depth() {
        let a = PrefixDigest::from_messages(&msgs(8));
        let b = PrefixDigest::from_messages(&msgs(8));
        assert_eq!(a.shared_depth(&b), 8);
    }

    #[test]
    fn a_continued_conversation_still_matches_its_own_past() {
        // Turn 5 of a session vs turn 9 of the same session: the first 4
        // messages are identical, so they share level index 2 (depth 4).
        let early = PrefixDigest::from_messages(&msgs(5));
        let later = PrefixDigest::from_messages(&msgs(9));
        assert_eq!(early.shared_depth(&later), 4);
    }

    #[test]
    fn divergent_conversations_do_not_match_deeply() {
        let mut left = msgs(8);
        let mut right = msgs(8);
        left[2] = "branch-a".into();
        right[2] = "branch-b".into();
        let a = PrefixDigest::from_messages(&left);
        let b = PrefixDigest::from_messages(&right);
        // They agree on the first 2 messages and diverge at the third.
        assert_eq!(a.shared_depth(&b), 2);
    }

    #[test]
    fn unrelated_conversations_share_nothing() {
        let a = PrefixDigest::from_messages(&["alpha".to_string()]);
        let b = PrefixDigest::from_messages(&["beta".to_string()]);
        assert_eq!(a.shared_depth(&b), 0);
    }

    #[test]
    fn message_boundaries_are_not_ambiguous() {
        let a = PrefixDigest::from_messages(&["ab".to_string(), "c".to_string()]);
        let b = PrefixDigest::from_messages(&["a".to_string(), "bc".to_string()]);
        assert_ne!(a, b);
    }

    #[test]
    fn claude_code_headers_give_exact_identity() {
        let mut h = HashMap::new();
        h.insert("x-claude-code-session-id".into(), "sess-1".to_string());
        h.insert("x-claude-code-agent-id".into(), "agent-7".to_string());
        let mut idx = SessionIndex::new();
        let n = idx.resolve(&h, None);
        assert_eq!(n.root, "sess-1");
        assert_eq!(n.node, "agent-7");
        assert_eq!(n.parent.as_deref(), Some("sess-1"));
        assert!(n.source.is_declared());
        assert!(!n.is_root());
    }

    #[test]
    fn a_main_thread_request_is_its_own_root() {
        let mut h = HashMap::new();
        h.insert("x-claude-code-session-id".into(), "sess-1".to_string());
        let mut idx = SessionIndex::new();
        let n = idx.resolve(&h, None);
        assert!(n.is_root());
        assert_eq!(n.parent, None);
    }

    #[test]
    fn nested_subagents_keep_their_real_parent() {
        let mut h = HashMap::new();
        h.insert("x-claude-code-session-id".into(), "sess-1".to_string());
        h.insert("x-claude-code-agent-id".into(), "agent-9".to_string());
        h.insert("x-claude-code-parent-agent-id".into(), "agent-4".to_string());
        let mut idx = SessionIndex::new();
        let n = idx.resolve(&h, None);
        assert_eq!(n.parent.as_deref(), Some("agent-4"));
    }

    #[test]
    fn codex_metadata_yields_the_task_root() {
        let mut h = HashMap::new();
        h.insert(
            "x-codex-turn-metadata".into(),
            r#"{"session_id":"s-1","turn_id":"t-9","root_turn_id":"t-1","parent_turn_id":"t-4"}"#
                .to_string(),
        );
        let mut idx = SessionIndex::new();
        let n = idx.resolve(&h, None);
        assert_eq!(n.root, "t-1");
        assert_eq!(n.node, "t-9");
        assert_eq!(n.parent.as_deref(), Some("t-4"));
    }

    #[test]
    fn codex_metadata_falls_back_to_session_when_no_root() {
        let mut h = HashMap::new();
        h.insert("x-codex-turn-metadata".into(), r#"{"session_id":"s-1"}"#.to_string());
        let mut idx = SessionIndex::new();
        assert_eq!(idx.resolve(&h, None).root, "s-1");
    }

    #[test]
    fn malformed_codex_metadata_does_not_panic() {
        for bad in [r#"{"root_turn_id":"#, "{", "", "not json", r#"{"root_turn_id":null}"#] {
            let mut h = HashMap::new();
            h.insert("x-codex-turn-metadata".into(), bad.to_string());
            let mut idx = SessionIndex::new();
            let n = idx.resolve(&h, None);
            assert_eq!(n.source, IdentitySource::Anonymous);
        }
    }

    #[test]
    fn undeclared_requests_are_stitched_into_one_session() {
        let mut idx = SessionIndex::new();
        let h: HashMap<String, String> = HashMap::new();

        let turn3 = PrefixDigest::from_messages(&msgs(3));
        let first = idx.resolve(&h, Some(turn3));

        // Same conversation, four turns later.
        let turn7 = PrefixDigest::from_messages(&msgs(7));
        let second = idx.resolve(&h, Some(turn7));

        assert_eq!(first.root, second.root, "continuation should join its session");
        assert_eq!(idx.live_sessions(), 1);
    }

    #[test]
    fn an_unrelated_conversation_opens_its_own_session() {
        let mut idx = SessionIndex::new();
        let h: HashMap<String, String> = HashMap::new();
        idx.resolve(&h, Some(PrefixDigest::from_messages(&msgs(4))));

        let other: Vec<String> = (0..4).map(|i| format!("other-{i}")).collect();
        idx.resolve(&h, Some(PrefixDigest::from_messages(&other)));

        assert_eq!(idx.live_sessions(), 2);
    }

    #[test]
    fn a_shared_system_prompt_alone_does_not_merge_sessions() {
        // Both start with the same system message and diverge immediately.
        let mut idx = SessionIndex::new();
        let h: HashMap<String, String> = HashMap::new();
        idx.resolve(&h, Some(PrefixDigest::from_messages(&["sys".to_string(), "a".to_string()])));
        idx.resolve(&h, Some(PrefixDigest::from_messages(&["sys".to_string(), "b".to_string()])));
        assert_eq!(idx.live_sessions(), 2, "depth-1 agreement must not be enough");
    }

    #[test]
    fn confidence_reflects_the_evidence() {
        assert_eq!(
            IdentitySource::Declared { session: "s".into(), node: None, parent: None }.confidence(),
            1.0
        );
        assert_eq!(IdentitySource::Anonymous.confidence(), 0.0);
        let shallow = IdentitySource::Inferred { shared_depth: 2 }.confidence();
        let deep = IdentitySource::Inferred { shared_depth: 32 }.confidence();
        assert!(deep > shallow);
    }
}
