use hyper::header::HeaderMap;
use tailfin_ident::Headers;

/// Borrowed view so tailfin-ident stays free of HTTP types.
pub struct HeaderView<'a>(pub &'a HeaderMap);

impl Headers for HeaderView<'_> {
    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(|v| v.to_str().ok())
    }
}

/// Canonical message bytes for a prefix digest. `None` if the body has no
/// `messages` array — shadow mode then records "no digest."
pub fn messages_from_body(body: &[u8]) -> Option<Vec<Vec<u8>>> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let msgs = v.get("messages")?.as_array()?;
    if msgs.is_empty() {
        return None;
    }
    Some(msgs.iter().map(canonical_message).collect())
}

fn canonical_message(m: &serde_json::Value) -> Vec<u8> {
    let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
    let mut out = Vec::with_capacity(role.len() + 16);
    out.extend_from_slice(role.as_bytes());
    out.push(0x1e);
    match m.get("content") {
        Some(serde_json::Value::String(s)) => out.extend_from_slice(s.as_bytes()),
        Some(other) => {
            if let Ok(bytes) = serde_json::to_vec(other) {
                out.extend(bytes);
            }
        }
        None => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tailfin_ident::PrefixDigest;

    #[test]
    fn messages_from_body_reads_anthropic_shape() {
        let body = br#"{"model":"x","messages":[{"role":"user","content":"hi"}]}"#;
        let msgs = messages_from_body(body).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], b"user\x1ehi");
    }

    #[test]
    fn empty_or_non_json_yields_no_digest() {
        assert!(messages_from_body(b"{}").is_none());
        assert!(messages_from_body(b"not json").is_none());
        assert!(messages_from_body(b"").is_none());
    }

    #[test]
    fn continued_conversation_shares_prefix() {
        let early = messages_from_body(
            br#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"}]}"#,
        )
        .unwrap();
        let later = messages_from_body(
            br#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"},{"role":"user","content":"c"}]}"#,
        )
        .unwrap();
        let a = PrefixDigest::from_messages(&early);
        let b = PrefixDigest::from_messages(&later);
        assert_eq!(a.shared_depth(&b), 2);
    }
}
