//! Task tree arena with live cost roll-up.
//!
//! The budget unit is the **root task**, not the API key, the user, or the
//! calendar month. Every product that enforces a ceiling today picks one of
//! those three, which is why a runaway fan-out is invisible to all of them:
//! eight subagents on one key look exactly like ordinary traffic.
//!
//! This arena is deliberately bounded and in-memory. A single developer's
//! machine does not need Postgres to answer "what has this task cost so far",
//! and requiring one is the thing that keeps existing gateways out of reach.

#![forbid(unsafe_code)]

use tailfin_ident::NodeRef;
use tailfin_wire::{RateCard, Usage};
use std::collections::HashMap;

/// Outcome of admitting a request against a task's ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Under budget. Relay it.
    Allow,
    /// This request crosses the ceiling. Relay it, then seal the task.
    ///
    /// Cost is only knowable after a response completes, so the crossing
    /// request always lands. Overshoot is bounded by one in-flight request per
    /// branch — the same bound Anthropic's own session budgets carry. Anyone
    /// claiming a hard ceiling without this caveat is not measuring.
    Last,
    /// The task is already sealed. Stop it.
    Deny {
        spent_micros: u64,
        ceiling_micros: u64,
    },
}

/// How a sealed task should be stopped.
///
/// Layered on purpose. A synthetic `end_turn` ends the current turn cleanly and
/// leaves a readable summary in the transcript; the hard status stops the
/// *next* request. Using only the first lets spending resume on the next turn.
/// Using only the second throws away in-progress work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopStyle {
    /// Valid response, `stop_reason: end_turn`, no tool_use block. The agent
    /// finishes its turn and summarizes.
    SyntheticEndTurn,
    /// `429` plus `x-should-retry: false`. Claude Code stops immediately;
    /// without that header it retries up to ten times with backoff.
    RetryableFalse429,
    /// `402`. The clean stop for anything on the Vercel AI SDK — opencode,
    /// Cline, Continue. They treat 429 as retryable and 402 as terminal.
    PaymentRequired402,
}

#[derive(Clone, Debug)]
struct Node {
    id: String,
    parent: Option<String>,
    label: Option<String>,
    usage: Usage,
    requests: u32,
    incomplete: u32,
    depth: u8,
    /// Voucher minted from the parent. `None` means inherit the root ceiling.
    allowance_micros: Option<u64>,
    sealed: bool,
}

/// One root task and everything spawned beneath it.
#[derive(Clone, Debug)]
pub struct Task {
    pub root: String,
    nodes: HashMap<String, Node>,
    order: Vec<String>,
    ceiling_micros: Option<u64>,
    sealed: bool,
    peak_concurrency: u32,
    live: u32,
    /// 0.0–1.0. Each new child is minted this fraction of the parent's remaining voucher.
    voucher_share: Option<f64>,
    synthetic_sent: bool,
}

impl Task {
    pub fn new(root: impl Into<String>) -> Self {
        let root = root.into();
        let mut nodes = HashMap::new();
        nodes.insert(
            root.clone(),
            Node {
                id: root.clone(),
                parent: None,
                label: None,
                usage: Usage::default(),
                requests: 0,
                incomplete: 0,
                depth: 0,
                allowance_micros: None,
                sealed: false,
            },
        );
        Self {
            order: vec![root.clone()],
            root,
            nodes,
            ceiling_micros: None,
            sealed: false,
            peak_concurrency: 0,
            live: 0,
            voucher_share: None,
            synthetic_sent: false,
        }
    }

    pub fn with_ceiling_micros(mut self, micros: u64) -> Self {
        self.ceiling_micros = Some(micros);
        if let Some(n) = self.nodes.get_mut(&self.root) {
            n.allowance_micros = Some(micros);
        }
        self
    }

    pub fn with_voucher_share(mut self, share: f64) -> Self {
        self.voucher_share = Some(share.clamp(0.0, 1.0));
        self
    }

    fn ensure(&mut self, node: &str, parent: Option<&str>, label: Option<&str>) {
        if self.nodes.contains_key(node) {
            if let (Some(l), Some(n)) = (label, self.nodes.get_mut(node)) {
                n.label.get_or_insert_with(|| l.to_string());
            }
            return;
        }
        let depth = parent
            .and_then(|p| self.nodes.get(p))
            .map(|p| p.depth.saturating_add(1))
            .unwrap_or(1);
        self.nodes.insert(
            node.to_string(),
            Node {
                id: node.to_string(),
                parent: parent.map(str::to_string),
                label: label.map(str::to_string),
                usage: Usage::default(),
                requests: 0,
                incomplete: 0,
                depth,
                allowance_micros: None,
                sealed: false,
            },
        );
        self.order.push(node.to_string());
    }

    /// Called when a request begins. Tracks concurrency, which is what makes a
    /// fan-out visible as it happens rather than in hindsight.
    pub fn begin(&mut self, at: &NodeRef, label: Option<&str>) {
        self.ensure(&at.node, at.parent.as_deref(), label);
        self.mint_voucher(at);
        self.live += 1;
        self.peak_concurrency = self.peak_concurrency.max(self.live);
    }

    fn mint_voucher(&mut self, at: &NodeRef) {
        let Some(share) = self.voucher_share else {
            return;
        };
        if at.is_root() {
            return;
        }
        if self
            .nodes
            .get(&at.node)
            .and_then(|n| n.allowance_micros)
            .is_some()
        {
            return;
        }
        let parent_id = at.parent.as_deref().unwrap_or(self.root.as_str());
        let parent_allow = self.allowance_of(parent_id);
        let minted: u64 = self
            .children_of(parent_id)
            .into_iter()
            .filter(|c| *c != at.node.as_str())
            .filter_map(|c| self.nodes.get(c).and_then(|n| n.allowance_micros))
            .sum();
        let remaining = parent_allow.saturating_sub(minted);
        let allot = (remaining as f64 * share).floor() as u64;
        if let Some(n) = self.nodes.get_mut(&at.node) {
            n.allowance_micros = Some(allot);
        }
    }

    pub fn allowance_of(&self, id: &str) -> u64 {
        self.nodes
            .get(id)
            .and_then(|n| n.allowance_micros)
            .or(self.ceiling_micros)
            .unwrap_or(0)
    }

    pub fn subtree_spent_micros(&self, id: &str, rates: &RateCard) -> u64 {
        let mut total = 0u64;
        let mut stack = vec![id];
        let mut seen = std::collections::HashSet::new();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur) {
                continue;
            }
            if let Some(n) = self.nodes.get(cur) {
                total = total.saturating_add(n.usage.micros(rates));
            }
            for c in self.children_of(cur) {
                stack.push(c);
            }
        }
        total
    }

    /// Admit a request at a specific node. Root ceiling still binds the tree.
    pub fn admit_at(&mut self, at: &NodeRef, rates: &RateCard) -> Admission {
        let root_ad = self.admit(rates);
        if !matches!(root_ad, Admission::Allow) {
            return root_ad;
        }
        let cap = self.allowance_of(&at.node);
        if cap == 0 && self.ceiling_micros.is_none() {
            return Admission::Allow;
        }
        let spent = self.subtree_spent_micros(&at.node, rates);
        let node_sealed = self.nodes.get(&at.node).map(|n| n.sealed).unwrap_or(false);
        if node_sealed {
            return Admission::Deny {
                spent_micros: spent,
                ceiling_micros: cap,
            };
        }
        if spent >= cap
            && self
                .nodes
                .get(&at.node)
                .and_then(|n| n.allowance_micros)
                .is_some()
        {
            if let Some(n) = self.nodes.get_mut(&at.node) {
                n.sealed = true;
            }
            return Admission::Last;
        }
        Admission::Allow
    }

    /// First stop is a synthetic end_turn; later stops are the hard status.
    pub fn take_stop(&mut self) -> StopStyle {
        if !self.synthetic_sent {
            self.synthetic_sent = true;
            StopStyle::SyntheticEndTurn
        } else {
            StopStyle::RetryableFalse429
        }
    }

    /// Called when a request finishes. `complete` is false when the terminal
    /// usage frame never arrived.
    pub fn finish(&mut self, at: &NodeRef, usage: &Usage, complete: bool) {
        self.ensure(&at.node, at.parent.as_deref(), None);
        if let Some(n) = self.nodes.get_mut(&at.node) {
            n.usage.merge(usage);
            n.requests += 1;
            if !complete {
                n.incomplete += 1;
            }
        }
        self.live = self.live.saturating_sub(1);
    }

    /// Decide whether a new request may proceed.
    pub fn admit(&mut self, rates: &RateCard) -> Admission {
        let Some(ceiling) = self.ceiling_micros else {
            return Admission::Allow;
        };
        let spent = self.spent_micros(rates);
        if self.sealed {
            return Admission::Deny {
                spent_micros: spent,
                ceiling_micros: ceiling,
            };
        }
        if spent >= ceiling {
            self.sealed = true;
            return Admission::Last;
        }
        Admission::Allow
    }

    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    pub fn total_usage(&self) -> Usage {
        let mut u = Usage::default();
        for n in self.nodes.values() {
            u.merge(&n.usage);
        }
        u
    }

    pub fn spent_micros(&self, rates: &RateCard) -> u64 {
        self.nodes.values().map(|n| n.usage.micros(rates)).sum()
    }

    pub fn root_usage(&self) -> Usage {
        self.nodes
            .get(&self.root)
            .map(|n| n.usage)
            .unwrap_or_default()
    }

    /// Usage of everything that is not the root — the work the user never saw.
    pub fn subagent_usage(&self) -> Usage {
        let mut u = Usage::default();
        for n in self.nodes.values().filter(|n| n.id != self.root) {
            u.merge(&n.usage);
        }
        u
    }

    /// Total tokens divided by main-thread tokens.
    ///
    /// This is the headline number, and no existing tool reports it: they read
    /// the parent transcript, which contains only each subagent's compact
    /// result, so fan-out looks like cheap parallelism.
    pub fn fan_out_multiplier(&self) -> Option<f64> {
        let root = self.root_usage().total();
        (root > 0).then(|| self.total_usage().total() as f64 / root as f64)
    }

    /// Share of spend that happened below the root, 0.0–1.0.
    pub fn subagent_share(&self, rates: &RateCard) -> Option<f64> {
        let total = self.spent_micros(rates);
        (total > 0).then(|| {
            let sub: u64 = self
                .nodes
                .values()
                .filter(|n| n.id != self.root)
                .map(|n| n.usage.micros(rates))
                .sum();
            sub as f64 / total as f64
        })
    }

    pub fn peak_concurrency(&self) -> u32 {
        self.peak_concurrency
    }

    pub fn max_depth(&self) -> u8 {
        self.nodes.values().map(|n| n.depth).max().unwrap_or(0)
    }

    /// Requests whose terminal usage frame never arrived. Surfaced rather than
    /// buried: a report that silently treats these as zero under-counts spend.
    pub fn incomplete_requests(&self) -> u32 {
        self.nodes.values().map(|n| n.incomplete).sum()
    }

    /// Direct children of a node, in first-seen order.
    pub fn children_of(&self, id: &str) -> Vec<&str> {
        self.order
            .iter()
            .filter_map(|k| self.nodes.get(k))
            .filter(|n| n.parent.as_deref() == Some(id))
            .map(|n| n.id.as_str())
            .collect()
    }

    /// Walk the tree depth-first from the root, yielding `(id, depth)`.
    ///
    /// Cycles are impossible by construction (a node's parent is fixed at
    /// creation and must already exist), but the visited set makes that
    /// explicit rather than load-bearing on an invariant nobody rechecks.
    pub fn walk(&self) -> Vec<(&str, u8)> {
        let mut out = Vec::new();
        let mut stack = vec![self.root.as_str()];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(n) = self.nodes.get(id) {
                out.push((n.id.as_str(), n.depth));
            }
            for c in self.children_of(id).into_iter().rev() {
                stack.push(c);
            }
        }
        out
    }

    /// Nodes ordered by cost, most expensive first. What the report prints.
    pub fn by_cost(&self, rates: &RateCard) -> Vec<NodeReport> {
        let mut v: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .map(|n| NodeReport {
                id: n.id.clone(),
                label: n.label.clone(),
                depth: n.depth,
                is_root: n.id == self.root,
                usage: n.usage,
                micros: n.usage.micros(rates),
                requests: n.requests,
            })
            .collect();
        v.sort_by(|a, b| b.micros.cmp(&a.micros).then_with(|| a.id.cmp(&b.id)));
        v
    }
}

/// A row in the fan-out report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeReport {
    pub id: String,
    pub label: Option<String>,
    pub depth: u8,
    pub is_root: bool,
    pub usage: Usage,
    pub micros: u64,
    pub requests: u32,
}

/// All live tasks.
#[derive(Default)]
pub struct Arena {
    tasks: HashMap<String, Task>,
    default_ceiling: Option<u64>,
    voucher_share: Option<f64>,
}

impl Arena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_default_ceiling_micros(mut self, micros: u64) -> Self {
        self.default_ceiling = Some(micros);
        self
    }

    pub fn with_voucher_share(mut self, share: f64) -> Self {
        self.voucher_share = Some(share.clamp(0.0, 1.0));
        self
    }

    pub fn task_mut(&mut self, root: &str) -> &mut Task {
        let default = self.default_ceiling;
        let share = self.voucher_share;
        self.tasks.entry(root.to_string()).or_insert_with(|| {
            let mut t = Task::new(root);
            if let Some(c) = default {
                t = t.with_ceiling_micros(c);
            }
            if let Some(s) = share {
                t = t.with_voucher_share(s);
            }
            t
        })
    }

    pub fn get(&self, root: &str) -> Option<&Task> {
        self.tasks.get(root)
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn roots(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tailfin_ident::IdentitySource;

    fn rates() -> RateCard {
        RateCard::from_base(15_000_000, 75_000_000)
    }

    fn node(root: &str, id: &str, parent: Option<&str>) -> NodeRef {
        NodeRef {
            root: root.into(),
            node: id.into(),
            parent: parent.map(str::to_string),
            source: IdentitySource::Declared {
                session: root.into(),
                node: Some(id.into()),
                parent: parent.map(str::to_string),
            },
        }
    }

    fn out(n: u64) -> Usage {
        Usage {
            output: n,
            ..Default::default()
        }
    }

    #[test]
    fn a_lone_main_thread_has_no_fan_out() {
        let mut t = Task::new("s1");
        let r = node("s1", "s1", None);
        t.begin(&r, None);
        t.finish(&r, &out(1000), true);
        assert_eq!(t.fan_out_multiplier(), Some(1.0));
        assert_eq!(t.subagent_share(&rates()), Some(0.0));
    }

    #[test]
    fn subagent_spend_rolls_up_to_the_root_task() {
        let mut t = Task::new("s1");
        let main = node("s1", "s1", None);
        t.begin(&main, None);
        t.finish(&main, &out(1_000), true);

        for i in 0..4 {
            let sub = node("s1", &format!("a{i}"), Some("s1"));
            t.begin(&sub, Some("research"));
            t.finish(&sub, &out(2_000), true);
        }

        assert_eq!(t.total_usage().output, 9_000);
        assert_eq!(t.root_usage().output, 1_000);
        assert_eq!(t.subagent_usage().output, 8_000);
        assert_eq!(t.fan_out_multiplier(), Some(9.0));
        let share = t.subagent_share(&rates()).unwrap();
        assert!((share - 8.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn peak_concurrency_sees_the_burst() {
        let mut t = Task::new("s1");
        let subs: Vec<_> = (0..5)
            .map(|i| node("s1", &format!("a{i}"), Some("s1")))
            .collect();
        for s in &subs {
            t.begin(s, None);
        }
        assert_eq!(t.peak_concurrency(), 5);
        for s in &subs {
            t.finish(s, &out(10), true);
        }
        // Peak is a high-water mark; it does not decay.
        assert_eq!(t.peak_concurrency(), 5);
    }

    #[test]
    fn nesting_depth_is_tracked() {
        let mut t = Task::new("s1");
        let a = node("s1", "a", Some("s1"));
        let b = node("s1", "b", Some("a"));
        let c = node("s1", "c", Some("b"));
        for n in [&a, &b, &c] {
            t.begin(n, None);
            t.finish(n, &out(1), true);
        }
        assert_eq!(t.max_depth(), 3);
    }

    #[test]
    fn the_crossing_request_is_allowed_then_the_task_seals() {
        // Ceiling $1.00. Each call bills 75c of output.
        let mut t = Task::new("s1").with_ceiling_micros(1_000_000);
        let r = node("s1", "s1", None);

        assert_eq!(t.admit(&rates()), Admission::Allow);
        t.begin(&r, None);
        t.finish(&r, &out(10_000), true); // 750_000 micros

        assert_eq!(t.admit(&rates()), Admission::Allow, "still under");
        t.begin(&r, None);
        t.finish(&r, &out(10_000), true); // 1_500_000 total

        assert_eq!(
            t.admit(&rates()),
            Admission::Last,
            "crossing seals the task"
        );
        assert!(t.is_sealed());
        match t.admit(&rates()) {
            Admission::Deny {
                spent_micros,
                ceiling_micros,
            } => {
                assert_eq!(ceiling_micros, 1_000_000);
                assert!(spent_micros >= 1_500_000);
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn a_task_without_a_ceiling_is_never_denied() {
        let mut t = Task::new("s1");
        let r = node("s1", "s1", None);
        t.begin(&r, None);
        t.finish(&r, &out(10_000_000), true);
        assert_eq!(t.admit(&rates()), Admission::Allow);
    }

    #[test]
    fn concurrent_subagents_bound_the_overshoot() {
        // Five in flight when the ceiling trips: at most five requests land
        // past it. This is the honest bound, and it must be stated.
        let mut t = Task::new("s1").with_ceiling_micros(100_000);
        let subs: Vec<_> = (0..5)
            .map(|i| node("s1", &format!("a{i}"), Some("s1")))
            .collect();
        for s in &subs {
            assert_eq!(t.admit(&rates()), Admission::Allow);
            t.begin(s, None);
        }
        for s in &subs {
            t.finish(s, &out(1_000), true); // 75_000 micros each
        }
        assert!(t.spent_micros(&rates()) > 100_000);
        assert_eq!(t.admit(&rates()), Admission::Last);
    }

    #[test]
    fn incomplete_requests_are_counted_not_hidden() {
        let mut t = Task::new("s1");
        let r = node("s1", "s1", None);
        t.begin(&r, None);
        t.finish(&r, &out(50), false);
        assert_eq!(t.incomplete_requests(), 1);
    }

    #[test]
    fn report_ranks_by_cost_and_marks_the_root() {
        let mut t = Task::new("s1");
        let main = node("s1", "s1", None);
        t.begin(&main, None);
        t.finish(&main, &out(100), true);

        let big = node("s1", "a1", Some("s1"));
        t.begin(&big, Some("prior art: spend caps"));
        t.finish(&big, &out(9_000), true);

        let rows = t.by_cost(&rates());
        assert_eq!(rows[0].id, "a1");
        assert_eq!(rows[0].label.as_deref(), Some("prior art: spend caps"));
        assert!(!rows[0].is_root);
        assert!(rows[1].is_root);
        assert!(rows[0].micros > rows[1].micros);
    }

    #[test]
    fn the_tree_walks_from_root_through_its_branches() {
        let mut t = Task::new("s1");
        let a = node("s1", "a", Some("s1"));
        let b = node("s1", "b", Some("s1"));
        let a1 = node("s1", "a1", Some("a"));
        for n in [&a, &b, &a1] {
            t.begin(n, None);
            t.finish(n, &out(1), true);
        }
        assert_eq!(t.children_of("s1"), vec!["a", "b"]);
        assert_eq!(t.children_of("a"), vec!["a1"]);
        assert!(t.children_of("b").is_empty());

        let walked = t.walk();
        assert_eq!(walked.len(), 4);
        assert_eq!(walked[0], ("s1", 0));
        // a's subtree is visited before its sibling b
        let ids: Vec<_> = walked.iter().map(|(i, _)| *i).collect();
        assert_eq!(ids, vec!["s1", "a", "a1", "b"]);
    }

    #[test]
    fn arena_applies_a_default_ceiling_to_new_tasks() {
        let mut a = Arena::new().with_default_ceiling_micros(5_000_000);
        let r = node("s9", "s9", None);
        {
            let t = a.task_mut("s9");
            t.begin(&r, None);
            t.finish(&r, &out(100_000), true); // $7.50
        }
        assert_eq!(a.task_mut("s9").admit(&rates()), Admission::Last);
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn voucher_children_cannot_sum_past_parent_remaining() {
        let mut t = Task::new("s1")
            .with_ceiling_micros(1_000_000)
            .with_voucher_share(0.3);
        let a = node("s1", "a", Some("s1"));
        let b = node("s1", "b", Some("s1"));
        t.begin(&a, None);
        t.begin(&b, None);
        let aa = t.allowance_of("a");
        let bb = t.allowance_of("b");
        assert_eq!(aa, 300_000);
        assert_eq!(bb, 210_000);
        assert!(aa + bb <= 1_000_000);
        // Even a greedy 100% share on a grandchild stays inside the child's voucher.
        let mut t = Task::new("s1")
            .with_ceiling_micros(1_000_000)
            .with_voucher_share(1.0);
        t.begin(&a, None);
        let c = node("s1", "c", Some("a"));
        t.begin(&c, None);
        assert_eq!(t.allowance_of("a"), 1_000_000);
        assert_eq!(t.allowance_of("c"), 1_000_000);
        assert!(t.allowance_of("a") + t.allowance_of("s1") >= t.allowance_of("c"));
    }

    #[test]
    fn admit_at_seals_a_child_without_sealing_the_root() {
        let mut t = Task::new("s1")
            .with_ceiling_micros(1_000_000)
            .with_voucher_share(0.3);
        let a = node("s1", "a", Some("s1"));
        t.begin(&a, None);
        t.finish(&a, &out(5_000), true); // 375_000 micros > 300_000 voucher
        assert_eq!(t.admit_at(&a, &rates()), Admission::Last);
        match t.admit_at(&a, &rates()) {
            Admission::Deny { .. } => {}
            other => panic!("{other:?}"),
        }
        let main = node("s1", "s1", None);
        assert_eq!(t.admit_at(&main, &rates()), Admission::Allow);
        assert!(!t.is_sealed());
    }

    #[test]
    fn take_stop_is_synthetic_then_hard() {
        let mut t = Task::new("s1").with_ceiling_micros(1);
        assert_eq!(t.take_stop(), StopStyle::SyntheticEndTurn);
        assert_eq!(t.take_stop(), StopStyle::RetryableFalse429);
        assert_eq!(t.take_stop(), StopStyle::RetryableFalse429);
    }

    #[test]
    fn cache_reads_are_priced_far_below_writes_in_a_roll_up() {
        let mut t = Task::new("s1");
        let r = node("s1", "s1", None);
        t.begin(&r, None);
        t.finish(
            &r,
            &Usage {
                cache_read: 1_000_000,
                cache_write_1h: 1_000_000,
                ..Default::default()
            },
            true,
        );
        let u = t.total_usage();
        assert_eq!(u.cache_read_write_ratio(), Some(1.0));
        // 1.5 + 30.0 dollars
        assert_eq!(t.spent_micros(&rates()), 31_500_000);
    }
}
