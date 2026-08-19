//! Conflict detector for LiteLLM / gateway configs. Rules cite published measurements.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub citation: &'static str,
    pub detail: String,
}

pub fn diagnose(config: &str) -> Vec<Finding> {
    let stripped: String = config
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let lower = stripped.to_ascii_lowercase();
    let mut out = Vec::new();

    let has_fallback = lower.contains("fallback")
        || lower.contains("fallbacks")
        || lower.contains("budget_fallback");
    let has_floor = lower.contains("tier_floor")
        || lower.contains("min_tier")
        || lower.contains("lowest_model")
        || lower.contains("floor:");
    if has_fallback && !has_floor {
        out.push(Finding {
            rule: "budget-fallback-no-floor",
            citation: "LiteLLM budget fallbacks have no floor (project verification; docs/architecture.md risks)",
            detail: "fallback chain present without tier_floor / min_tier".into(),
        });
    }

    let compress = lower.contains("compression")
        || lower.contains("compact_ratio")
        || lower.contains("context_compression");
    let aggressive = lower.contains("compression_ratio")
        || lower.contains("ratio: 0.")
        || lower.contains("ratio:0.")
        || extract_ratio(&lower).map(|r| r < 0.5).unwrap_or(false);
    if compress && aggressive {
        out.push(Finding {
            rule: "compression-evicts-cache-prefix",
            citation: "cache reads price at 0.1x input; 1h writes at 2x — compacting the prefix forces re-write (CLAUDE.md invariant 3; docs/architecture.md)",
            detail: "compression ratio will evict the persistent cache prefix".into(),
        });
    }

    let memory = lower.contains("memory") && (lower.contains("inject") || lower.contains("plugin"));
    let strip = lower.contains("strip")
        && (lower.contains("memory") || lower.contains("compression") || lower.contains("context"));
    if memory && strip {
        out.push(Finding {
            rule: "memory-inject-feeds-compression-strip",
            citation: "memory-inject feeding compression-strip is a published collision (docs/architecture.md weeks 5–8 doctor)",
            detail: "memory injection is paired with compression/strip".into(),
        });
    }
    out
}

fn extract_ratio(lower: &str) -> Option<f64> {
    for key in ["compression_ratio:", "compact_ratio:", "ratio:"] {
        if let Some(i) = lower.find(key) {
            let rest = lower[i + key.len()..].trim_start();
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(v) = num.parse::<f64>() {
                return Some(v);
            }
        }
    }
    None
}

pub fn render_doctor(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "tailfin doctor: no collisions\n".into();
    }
    let mut s = format!("tailfin doctor: {} finding(s)\n", findings.len());
    for f in findings {
        s.push_str(&format!(
            "- [{}] {}\n  cite: {}\n",
            f.rule, f.detail, f.citation
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
router_settings:
  fallbacks:
    - gpt-4: [gpt-3.5]
  # no floor key on purpose
general_settings:
  context_compression: true
  compression_ratio: 0.1
  memory_plugin:
    inject: true
  compression:
    strip: true
"#;

    #[test]
    fn fixture_triggers_all_three_rule_classes_with_citations() {
        let f = diagnose(FIXTURE);
        let rules: Vec<_> = f.iter().map(|x| x.rule).collect();
        assert!(rules.contains(&"budget-fallback-no-floor"), "{rules:?}");
        assert!(
            rules.contains(&"compression-evicts-cache-prefix"),
            "{rules:?}"
        );
        assert!(
            rules.contains(&"memory-inject-feeds-compression-strip"),
            "{rules:?}"
        );
        let text = render_doctor(&f);
        for row in &f {
            assert!(text.contains(row.citation), "{text}");
            assert!(!row.citation.is_empty());
        }
    }

    #[test]
    fn clean_config_is_silent() {
        let text = render_doctor(&diagnose("model: gpt-4\n"));
        assert!(text.contains("no collisions"), "{text}");
    }

    #[test]
    fn committed_fixture_file_triggers_the_three_rules() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/gateway-litellm.yaml");
        let raw = std::fs::read_to_string(&path).expect("fixture");
        let f = diagnose(&raw);
        assert_eq!(f.len(), 3, "{f:?}");
    }
}
