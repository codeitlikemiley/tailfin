//! HTTP relay. Observation is a tee off this path; this crate never owns a
//! response body to inspect it.

#![forbid(unsafe_code)]

mod hop;
mod proxy;
mod serve;

pub use hop::strip_hop_by_hop;
pub use proxy::{Proxy, RelayBody};
pub use serve::serve;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use hyper::Uri;

pub const DEFAULT_LISTEN: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 7171));

#[derive(Debug)]
pub enum Error {
    InvalidUpstream(&'static str),
    Config(String),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidUpstream(msg) => write!(f, "invalid upstream: {msg}"),
            Error::Config(msg) => write!(f, "{msg}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    pub upstream: Uri,
}

impl Config {
    /// Flags override env. `RAZ_LISTEN` / `--listen`, `RAZ_UPSTREAM` / `--upstream`.
    pub fn parse(
        args: impl IntoIterator<Item = impl AsRef<str>>,
        listen_env: Option<&str>,
        upstream_env: Option<&str>,
    ) -> Result<Self, Error> {
        let mut listen = listen_env.unwrap_or("127.0.0.1:7171").to_string();
        let mut upstream = upstream_env.map(str::to_string);
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_ref() {
                "--listen" => {
                    listen = args
                        .next()
                        .ok_or_else(|| Error::Config("--listen needs a value".into()))?
                        .as_ref()
                        .to_string();
                }
                "--upstream" => {
                    upstream = Some(
                        args.next()
                            .ok_or_else(|| Error::Config("--upstream needs a value".into()))?
                            .as_ref()
                            .to_string(),
                    );
                }
                "-h" | "--help" => return Err(Error::Config("help".into())),
                other => {
                    return Err(Error::Config(format!("unknown argument: {other}")));
                }
            }
        }
        let Some(upstream) = upstream else {
            return Err(Error::Config(
                "upstream required (--upstream or RAZ_UPSTREAM)".into(),
            ));
        };
        let listen: SocketAddr = listen
            .parse()
            .map_err(|_| Error::Config(format!("invalid listen address: {listen}")))?;
        let upstream: Uri = upstream
            .parse()
            .map_err(|_| Error::Config(format!("invalid upstream URI: {upstream}")))?;
        Ok(Self { listen, upstream })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::body::Incoming;
    use hyper::header::{HeaderMap, HeaderValue};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::{oneshot, Notify};

    #[test]
    fn parse_defaults_listen_to_loopback_7171() {
        let c = Config::parse(["--upstream", "http://example.com"], None, None).unwrap();
        assert_eq!(c.listen, DEFAULT_LISTEN);
        assert_eq!(c.listen.port(), 7171);
        assert!(c.listen.ip().is_loopback());
    }

    #[test]
    fn parse_requires_upstream() {
        let err = Config::parse(Vec::<&str>::new(), None, None).unwrap_err();
        assert!(err.to_string().contains("upstream"));
    }

    #[test]
    fn parse_flags_override_env() {
        let c = Config::parse(
            [
                "--listen",
                "127.0.0.1:9000",
                "--upstream",
                "http://flag.example",
            ],
            Some("127.0.0.1:8000"),
            Some("http://env.example"),
        )
        .unwrap();
        assert_eq!(c.listen.port(), 9000);
        assert_eq!(c.upstream.host(), Some("flag.example"));
    }

    struct Collected {
        status: StatusCode,
        headers: HeaderMap,
        body: Bytes,
    }

    async fn spawn_http_server<H, Fut>(handler: H) -> std::net::SocketAddr
    where
        H: Fn(Request<Bytes>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Response<Full<Bytes>>> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let io = TokioIo::new(stream);
                let handler = handler.clone();
                let conn = http1::Builder::new().serve_connection(
                    io,
                    service_fn(move |req: Request<Incoming>| {
                        let handler = handler.clone();
                        async move {
                            let (parts, body) = req.into_parts();
                            let bytes = body.collect().await.expect("stub body").to_bytes();
                            let captured = Request::from_parts(parts, bytes);
                            Ok::<_, Infallible>(handler(captured).await)
                        }
                    }),
                );
                tokio::spawn(async move {
                    let _ = conn.await;
                });
            }
        });
        addr
    }

    async fn spawn_proxy(
        upstream: std::net::SocketAddr,
    ) -> (
        std::net::SocketAddr,
        oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let uri: Uri = format!("http://{upstream}").parse().unwrap();
        let proxy = Proxy::new(uri).unwrap();
        let (tx, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            serve(listener, proxy, async {
                let _ = rx.await;
            })
            .await
            .unwrap();
        });
        (addr, tx, handle)
    }

    async fn send(
        dest: std::net::SocketAddr,
        method: Method,
        path: &str,
        headers: HeaderMap,
        body: Bytes,
    ) -> Collected {
        let client = Client::builder(TokioExecutor::new()).build_http();
        let mut req = Request::builder()
            .method(method)
            .uri(format!("http://{dest}{path}"))
            .body(Full::new(body))
            .unwrap();
        for (k, v) in headers {
            if let Some(k) = k {
                req.headers_mut().insert(k, v);
            }
        }
        let resp = client.request(req).await.expect("client request");
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        Collected {
            status,
            headers,
            body,
        }
    }

    #[tokio::test]
    async fn non_streaming_json_round_trip_is_byte_identical_to_direct() {
        let payload =
            Bytes::from_static(br#"{"id":"msg_1","content":[{"type":"text","text":"hi"}]}"#);
        let stub = spawn_http_server({
            let payload = payload.clone();
            move |_req| {
                let payload = payload.clone();
                async move {
                    Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .header("x-request-id", "req_fixed")
                        .body(Full::new(payload))
                        .unwrap()
                }
            }
        })
        .await;

        let req_body = Bytes::from_static(br#"{"model":"claude-opus-4","messages":[]}"#);
        let (proxy_addr, shutdown, handle) = spawn_proxy(stub).await;
        let through = send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            HeaderMap::new(),
            req_body.clone(),
        )
        .await;
        shutdown.send(()).ok();
        handle.await.unwrap();

        let direct = send(
            stub,
            Method::POST,
            "/v1/messages",
            HeaderMap::new(),
            req_body,
        )
        .await;

        assert_eq!(through.status, StatusCode::OK);
        assert_eq!(through.status, direct.status);
        assert_eq!(through.body, payload);
        assert_eq!(through.body, direct.body);
        assert_eq!(
            through
                .headers
                .get("content-type")
                .map(HeaderValue::as_bytes),
            Some(&b"application/json"[..])
        );
        assert_eq!(
            through
                .headers
                .get("x-request-id")
                .map(HeaderValue::as_bytes),
            Some(&b"req_fixed"[..])
        );
    }

    #[tokio::test]
    async fn forwards_method_path_query_and_body() {
        let seen: Arc<Mutex<Option<(String, String, Bytes)>>> = Arc::new(Mutex::new(None));
        let stub = spawn_http_server({
            let seen = seen.clone();
            move |req| {
                let seen = seen.clone();
                async move {
                    *seen.lock().unwrap() = Some((
                        req.method().to_string(),
                        req.uri().to_string(),
                        req.body().clone(),
                    ));
                    Response::new(Full::new(Bytes::from_static(b"{}")))
                }
            }
        })
        .await;

        let body = Bytes::from_static(br#"{"model":"claude"}"#);
        let (proxy_addr, shutdown, handle) = spawn_proxy(stub).await;
        let resp = send(
            proxy_addr,
            Method::POST,
            "/v1/messages?beta=true",
            HeaderMap::new(),
            body.clone(),
        )
        .await;
        shutdown.send(()).ok();
        handle.await.unwrap();

        assert_eq!(resp.status, StatusCode::OK);
        let (method, uri, got_body) = seen.lock().unwrap().clone().expect("stub saw a request");
        assert_eq!(method, "POST");
        assert!(uri.contains("/v1/messages"), "path: {uri}");
        assert!(uri.contains("beta=true"), "query: {uri}");
        assert_eq!(got_body, body);
    }

    #[tokio::test]
    async fn hop_by_hop_request_headers_are_not_forwarded() {
        let seen: Arc<Mutex<Option<HeaderMap>>> = Arc::new(Mutex::new(None));
        let stub = spawn_http_server({
            let seen = seen.clone();
            move |req| {
                let seen = seen.clone();
                async move {
                    *seen.lock().unwrap() = Some(req.headers().clone());
                    Response::new(Full::new(Bytes::from_static(b"{}")))
                }
            }
        })
        .await;

        let mut headers = HeaderMap::new();
        headers.insert("connection", HeaderValue::from_static("close, x-secret"));
        headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        headers.insert("x-secret", HeaderValue::from_static("nope"));
        headers.insert("x-api-key", HeaderValue::from_static("sk-live"));
        headers.insert(
            "x-claude-code-session-id",
            HeaderValue::from_static("sess-1"),
        );

        let (proxy_addr, shutdown, handle) = spawn_proxy(stub).await;
        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            headers,
            Bytes::from_static(b"{}"),
        )
        .await;
        shutdown.send(()).ok();
        handle.await.unwrap();

        let got = seen.lock().unwrap().clone().expect("headers");
        assert!(
            got.get("x-secret").is_none(),
            "connection-listed header leaked"
        );
        assert!(got.get("keep-alive").is_none());
        assert_eq!(
            got.get("x-api-key").map(HeaderValue::as_bytes),
            Some(&b"sk-live"[..])
        );
        assert_eq!(
            got.get("x-claude-code-session-id")
                .map(HeaderValue::as_bytes),
            Some(&b"sess-1"[..])
        );
    }

    #[tokio::test]
    async fn hop_by_hop_response_headers_are_not_forwarded() {
        let stub = spawn_http_server(|_req| async move {
            Response::builder()
                .status(200)
                .header("connection", "close, x-secret")
                .header("x-secret", "nope")
                .header("x-real", "yes")
                .body(Full::new(Bytes::from_static(b"{}")))
                .unwrap()
        })
        .await;

        let (proxy_addr, shutdown, handle) = spawn_proxy(stub).await;
        let through = send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            HeaderMap::new(),
            Bytes::from_static(b"{}"),
        )
        .await;
        shutdown.send(()).ok();
        handle.await.unwrap();

        assert!(through.headers.get("x-secret").is_none());
        assert_eq!(
            through.headers.get("x-real").map(HeaderValue::as_bytes),
            Some(&b"yes"[..])
        );
    }

    #[tokio::test]
    async fn host_is_rewritten_to_upstream_authority() {
        let seen: Arc<Mutex<Option<HeaderValue>>> = Arc::new(Mutex::new(None));
        let stub = spawn_http_server({
            let seen = seen.clone();
            move |req| {
                let seen = seen.clone();
                async move {
                    *seen.lock().unwrap() = req.headers().get("host").cloned();
                    Response::new(Full::new(Bytes::from_static(b"{}")))
                }
            }
        })
        .await;

        let (proxy_addr, shutdown, handle) = spawn_proxy(stub).await;
        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            HeaderMap::new(),
            Bytes::from_static(b"{}"),
        )
        .await;
        shutdown.send(()).ok();
        handle.await.unwrap();

        let host = seen.lock().unwrap().clone().expect("host");
        let expected = stub.to_string();
        assert_eq!(
            host.as_bytes(),
            expected.as_bytes(),
            "host should be the stub, not the proxy"
        );
        assert_ne!(host.as_bytes(), proxy_addr.to_string().as_bytes());
    }

    #[tokio::test]
    async fn graceful_shutdown_completes_in_flight_request() {
        let received = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let stub = spawn_http_server({
            let received = received.clone();
            let release = release.clone();
            move |_req| {
                let received = received.clone();
                let release = release.clone();
                async move {
                    received.notify_one();
                    release.notified().await;
                    Response::new(Full::new(Bytes::from_static(br#"{"ok":true}"#)))
                }
            }
        })
        .await;

        let (proxy_addr, shutdown, handle) = spawn_proxy(stub).await;
        let client = tokio::spawn(async move {
            send(
                proxy_addr,
                Method::POST,
                "/v1/messages",
                HeaderMap::new(),
                Bytes::from_static(b"{}"),
            )
            .await
        });

        tokio::time::timeout(Duration::from_secs(2), received.notified())
            .await
            .expect("stub should see the request");
        shutdown.send(()).ok();
        tokio::task::yield_now().await;
        release.notify_one();

        let collected = tokio::time::timeout(Duration::from_secs(2), client)
            .await
            .expect("client should finish")
            .expect("join");
        assert_eq!(collected.body.as_ref(), br#"{"ok":true}"#);
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("proxy should shut down")
            .expect("join");
    }

    #[tokio::test]
    async fn unreachable_upstream_returns_502() {
        let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_addr = dead.local_addr().unwrap();
        drop(dead);

        let (proxy_addr, shutdown, handle) = spawn_proxy(dead_addr).await;
        let collected = send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            HeaderMap::new(),
            Bytes::from_static(b"{}"),
        )
        .await;
        shutdown.send(()).ok();
        handle.await.unwrap();
        assert_eq!(collected.status, StatusCode::BAD_GATEWAY);
        assert_eq!(collected.body.as_ref(), b"bad gateway\n");
    }
}
