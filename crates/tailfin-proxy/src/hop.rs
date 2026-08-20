use hyper::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::Method;

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

/// GET + `Upgrade: websocket` + `Connection: upgrade`. Codex prefers this for
/// `/v1/responses`; stripping Upgrade turns it into a plain GET that 404s.
pub fn is_websocket_upgrade(method: &Method, headers: &HeaderMap) -> bool {
    if method != Method::GET {
        return false;
    }
    let upgrade = headers
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.eq_ignore_ascii_case("websocket"));
    let connection = headers.get_all("connection").iter().any(|v| {
        v.to_str()
            .map(|s| {
                s.split(',')
                    .any(|t| t.trim().eq_ignore_ascii_case("upgrade"))
            })
            .unwrap_or(false)
    });
    upgrade && connection
}

/// Hop-by-hop strip that keeps the WebSocket handshake headers.
pub fn strip_hop_keeping_upgrade(headers: &mut HeaderMap) {
    let mut extra = Vec::new();
    for value in headers.get_all("connection") {
        let Ok(s) = value.to_str() else { continue };
        for token in s.split(',') {
            let token = token.trim();
            if token.is_empty() || token.eq_ignore_ascii_case("upgrade") {
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
        if *name == "connection" || *name == "upgrade" {
            continue;
        }
        headers.remove(*name);
    }
    // RFC 6455: hop must send Connection: Upgrade, not leftovers like keep-alive.
    headers.insert("connection", HeaderValue::from_static("Upgrade"));
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

    #[test]
    fn websocket_upgrade_is_detected_on_get() {
        let mut h = HeaderMap::new();
        h.insert("connection", hv("Upgrade"));
        h.insert("upgrade", hv("websocket"));
        h.insert("sec-websocket-key", hv("dGhlIHNhbXBsZSBub25jZQ=="));
        assert!(is_websocket_upgrade(&Method::GET, &h));
        assert!(!is_websocket_upgrade(&Method::POST, &h));
    }

    #[test]
    fn strip_hop_keeping_upgrade_preserves_handshake() {
        let mut h = HeaderMap::new();
        h.insert("connection", hv("keep-alive, Upgrade"));
        h.insert("upgrade", hv("websocket"));
        h.insert("keep-alive", hv("timeout=5"));
        h.insert("sec-websocket-key", hv("dGhlIHNhbXBsZSBub25jZQ=="));
        h.insert("sec-websocket-version", hv("13"));
        h.insert("x-codex-turn-metadata", hv("{}"));
        strip_hop_keeping_upgrade(&mut h);
        assert_eq!(
            h.get("upgrade").map(HeaderValue::as_bytes),
            Some(&b"websocket"[..])
        );
        assert_eq!(
            h.get("connection").map(HeaderValue::as_bytes),
            Some(&b"Upgrade"[..])
        );
        assert!(h.get("keep-alive").is_none());
        assert!(h.get("sec-websocket-key").is_some());
        assert!(h.get("x-codex-turn-metadata").is_some());
    }
}
