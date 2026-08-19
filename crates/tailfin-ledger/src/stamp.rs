//! Cost stamps and per-hunk blame. Opt-in; refused below capture-grade confidence.

use crate::{tasks_from_records, Record};
use tailfin_wire::RateCard;

/// Declared identity only. Inferred maxes at 0.99; anonymous is 0.
pub const CAPTURE_GRADE: f32 = 1.0;

pub fn stamp_allowed(records: &[Record]) -> bool {
    !records.is_empty() && records.iter().all(|r| r.confidence >= CAPTURE_GRADE)
}

/// One-line collapsed trailer. Expand by reading the ledger.
pub fn format_stamp(records: &[Record], rates: Option<&RateCard>) -> Result<String, String> {
    if !stamp_allowed(records) {
        return Err(
            "attribution below capture-grade (need declared identity, confidence 1.0)".into(),
        );
    }
    let tasks = tasks_from_records(records);
    let n = tasks.len();
    let incomplete: u32 = tasks.iter().map(|t| t.incomplete_requests()).sum();
    let fan = tasks
        .iter()
        .filter_map(|t| t.fan_out_multiplier())
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(1.0);
    let conf = records
        .iter()
        .map(|r| r.confidence)
        .fold(1.0f32, |a, b| a.min(b));
    let cost = match rates {
        Some(r) => {
            let micros: u64 = tasks.iter().map(|t| t.spent_micros(r)).sum();
            format!("${:.4}", micros as f64 / 1_000_000.0)
        }
        None => "tokens-only".into(),
    };
    Ok(format!(
        "Tailfin-Cost: tasks={n} cost={cost} fan-out={fan:.2}x incomplete={incomplete} conf={conf:.2} models=declared"
    ))
}

/// Per-node cost as hunk-shaped rows (path unknown without a git tree).
pub fn format_blame(records: &[Record], rates: Option<&RateCard>) -> String {
    let tasks = tasks_from_records(records);
    let mut s = String::from("hunk                                tokens      $\n");
    for task in &tasks {
        let rows = if let Some(r) = rates {
            task.by_cost(r)
                .into_iter()
                .map(|n| (n.id, n.usage.total(), Some(n.micros)))
                .collect::<Vec<_>>()
        } else {
            task.walk()
                .into_iter()
                .filter_map(|(id, _)| {
                    let usage = task
                        .by_cost(&RateCard::from_base(1, 1))
                        .into_iter()
                        .find(|n| n.id == id)
                        .map(|n| n.usage)?;
                    Some((id.to_string(), usage.total(), None))
                })
                .collect()
        };
        for (id, tokens, micros) in rows {
            let dollars = micros
                .map(|m| format!("{:.4}", m as f64 / 1_000_000.0))
                .unwrap_or_else(|| "-".into());
            let short = if id.len() > 28 {
                format!("{}…", &id[..27])
            } else {
                id
            };
            s.push_str(&format!("  {short:<32} {tokens:>7}  {dollars}\n"));
        }
    }
    if tasks.is_empty() {
        s.push_str("(no records)\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Record;
    use tailfin_ident::{IdentitySource, NodeRef};
    use tailfin_wire::Usage;

    fn rec(conf: f32) -> Record {
        let mut r = Record::from_finish(
            &NodeRef {
                root: "s".into(),
                node: "s".into(),
                parent: None,
                source: if conf >= 1.0 {
                    IdentitySource::Declared {
                        session: "s".into(),
                        node: None,
                        parent: None,
                    }
                } else {
                    IdentitySource::Anonymous
                },
            },
            &Usage {
                output: 100,
                ..Default::default()
            },
            false,
            1,
        );
        r.confidence = conf;
        r
    }

    #[test]
    fn stamp_refused_below_capture_grade() {
        let err = format_stamp(&[rec(0.0)], None).unwrap_err();
        assert!(err.contains("capture-grade"), "{err}");
    }

    #[test]
    fn stamp_line_has_required_fields() {
        let line = format_stamp(&[rec(1.0)], None).unwrap();
        for needle in [
            "Tailfin-Cost:",
            "tasks=",
            "cost=",
            "fan-out=",
            "incomplete=",
            "conf=",
        ] {
            assert!(line.contains(needle), "{line}");
        }
        assert!(!line.contains('\n'), "collapsed to one line: {line}");
    }

    #[test]
    fn blame_prints_per_node_cost() {
        let text = format_blame(&[rec(1.0)], None);
        assert!(text.contains("hunk"), "{text}");
        assert!(text.contains("tokens"), "{text}");
        assert!(text.contains("100") || text.contains("s"), "{text}");
    }
}
