use hyper::header::{HeaderMap, HeaderName};

/// Headers that describe one TCP hop, not the origin request/response.
///
/// RFC 9110 §7.6.1 plus the de-facto `proxy-connection` leftover. Tokens
/// listed in `Connection` are hop-by-hop too — a client can name any header
/// there and a compliant proxy must consume it.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Remove hop-by-hop headers in place. Remaining entries keep their order.
pub fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let mut extra = Vec::new();
    for value in headers.get_all("connection") {
        let Ok(s) = value.to_str() else { continue };
        for token in s.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if let Ok(name) = HeaderName::from_bytes(token.as_bytes()) {
                extra.push(name);
            }
        }
    }
    for name in extra {
        headers.remove(name);
    }
    for name in HOP_BY_HOP {
        headers.remove(*name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::{HeaderValue, AUTHORIZATION};

    fn hv(s: &str) -> HeaderValue {
        HeaderValue::from_str(s).unwrap()
    }

    #[test]
    fn strip_hop_by_hop_removes_standard_headers() {
        let mut h = HeaderMap::new();
        h.insert("connection", hv("keep-alive"));
        h.insert("keep-alive", hv("timeout=5"));
        h.insert("proxy-connection", hv("close"));
        h.insert("te", hv("trailers"));
        h.insert("trailer", hv("x-checksum"));
        h.insert("transfer-encoding", hv("chunked"));
        h.insert("upgrade", hv("websocket"));
        h.insert("proxy-authenticate", hv("Basic"));
        h.insert("proxy-authorization", hv("Basic abc"));
        h.insert("x-api-key", hv("secret"));

        strip_hop_by_hop(&mut h);

        for name in HOP_BY_HOP {
            assert!(h.get(*name).is_none(), "{name} must be stripped");
        }
        assert_eq!(
            h.get("x-api-key").map(HeaderValue::as_bytes),
            Some(&b"secret"[..])
        );
    }

    #[test]
    fn strip_hop_by_hop_removes_headers_listed_in_connection() {
        let mut h = HeaderMap::new();
        h.insert("connection", hv("close, x-secret, X-Also"));
        h.insert("x-secret", hv("nope"));
        h.insert("x-also", hv("nope"));
        h.insert("x-real", hv("yes"));

        strip_hop_by_hop(&mut h);

        assert!(h.get("x-secret").is_none());
        assert!(h.get("x-also").is_none());
        assert_eq!(
            h.get("x-real").map(HeaderValue::as_bytes),
            Some(&b"yes"[..])
        );
        assert!(h.get("connection").is_none());
    }

    #[test]
    fn strip_hop_by_hop_keeps_end_to_end_headers() {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, hv("Bearer tok"));
        h.insert("anthropic-version", hv("2023-06-01"));
        h.insert("x-api-key", hv("sk"));
        h.insert("x-claude-code-session-id", hv("sess-1"));
        h.insert("content-type", hv("application/json"));

        strip_hop_by_hop(&mut h);

        assert_eq!(h.len(), 5);
        assert_eq!(
            h.get(AUTHORIZATION).map(HeaderValue::as_bytes),
            Some(&b"Bearer tok"[..])
        );
        assert_eq!(
            h.get("x-claude-code-session-id").map(HeaderValue::as_bytes),
            Some(&b"sess-1"[..])
        );
    }
}
