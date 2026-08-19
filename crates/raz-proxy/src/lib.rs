//! HTTP relay. Observation is a tee off this path; this crate never owns a
//! response body to inspect it.

#![forbid(unsafe_code)]

mod hop;
mod identity;
mod proxy;
mod serve;
mod tee;

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
    use crate::proxy::{Meter, ShadowCmp};
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::body::Incoming;
    use hyper::body::{Body, Frame};
    use hyper::header::{HeaderMap, HeaderValue};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use raz_wire::Usage;
    use std::convert::Infallible;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::{mpsc, oneshot, Notify};

    #[test]
    fn parse_defaults_listen_to_loopback_7171() {
        let c = Config::parse(["--upstream", "http://example.com"], None, None).unwrap();
        assert_eq!(c.listen, DEFAULT_LISTEN);
        assert_eq!(c.listen.port(), 7171);
        assert!(c.listen.ip().is_loopback());
    }

    #[test]
    fn accepts_https_upstream() {
        Proxy::new("https://api.anthropic.com".parse().unwrap())
            .expect("https upstream should construct");
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

    struct MpscBody {
        rx: mpsc::Receiver<Bytes>,
    }

    impl Body for MpscBody {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            match self.rx.poll_recv(cx) {
                Poll::Ready(Some(data)) => Poll::Ready(Some(Ok(Frame::data(data)))),
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    async fn spawn_http_server<H, Fut, B>(handler: H) -> std::net::SocketAddr
    where
        H: Fn(Request<Bytes>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Response<B>> + Send + 'static,
        B: Body<Data = Bytes> + Send + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
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
        let (addr, tx, handle, _) = spawn_proxy_cfg(upstream, |p| p).await;
        (addr, tx, handle)
    }

    async fn spawn_proxy_cfg(
        upstream: std::net::SocketAddr,
        configure: impl FnOnce(Proxy) -> Proxy,
    ) -> (
        std::net::SocketAddr,
        oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
        Proxy,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let uri: Uri = format!("http://{upstream}").parse().unwrap();
        let proxy = configure(Proxy::new(uri).unwrap());
        let serving = proxy.clone();
        let (tx, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            serve(listener, serving, async {
                let _ = rx.await;
            })
            .await
            .unwrap();
        });
        (addr, tx, handle, proxy)
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

    #[tokio::test(flavor = "multi_thread")]
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

    #[tokio::test(flavor = "multi_thread")]
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

    #[tokio::test(flavor = "multi_thread")]
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

    #[tokio::test(flavor = "multi_thread")]
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

    #[tokio::test(flavor = "multi_thread")]
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

    #[tokio::test(flavor = "multi_thread")]
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

    #[tokio::test(flavor = "multi_thread")]
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

    const SSE_START: &[u8] = b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n";
    const SSE_DELTA: &[u8] =
        b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n";
    const SSE_END: &[u8] = b"event: message_delta\ndata: {\"type\":\"message_delta\"}\n\n";

    async fn sse_stub(first_sent: Arc<Notify>, send_rest: Arc<Notify>) -> std::net::SocketAddr {
        spawn_http_server(move |_req| {
            let first_sent = first_sent.clone();
            let send_rest = send_rest.clone();
            async move {
                let (tx, rx) = mpsc::channel(8);
                tokio::spawn(async move {
                    let _ = tx.send(Bytes::from_static(SSE_START)).await;
                    first_sent.notify_one();
                    send_rest.notified().await;
                    let _ = tx.send(Bytes::from_static(SSE_DELTA)).await;
                    let _ = tx.send(Bytes::from_static(SSE_END)).await;
                });
                Response::builder()
                    .status(200)
                    .header("content-type", "text/event-stream")
                    .header("cache-control", "no-cache")
                    .body(MpscBody { rx })
                    .unwrap()
            }
        })
        .await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sse_is_forwarded_unbuffered() {
        let first_sent = Arc::new(Notify::new());
        let send_rest = Arc::new(Notify::new());
        let stub = sse_stub(first_sent.clone(), send_rest.clone()).await;
        let (proxy_addr, shutdown, handle) = spawn_proxy(stub).await;

        let client = Client::builder(TokioExecutor::new()).build_http();
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("http://{proxy_addr}/v1/messages"))
            .header("accept", "text/event-stream")
            .body(Full::new(Bytes::from_static(br#"{"stream":true}"#)))
            .unwrap();
        let resp = client.request(req).await.expect("proxy request");
        assert_eq!(resp.status(), StatusCode::OK);
        let mut body = resp.into_body();

        tokio::time::timeout(Duration::from_secs(2), first_sent.notified())
            .await
            .expect("stub should emit the first SSE frame");

        let first = tokio::time::timeout(Duration::from_secs(2), body.frame())
            .await
            .expect("first frame should arrive before the stub continues")
            .expect("frame")
            .expect("ok")
            .into_data()
            .expect("data");
        assert_eq!(first.as_ref(), SSE_START);

        send_rest.notify_one();
        let rest = body.collect().await.expect("rest").to_bytes();
        let mut expected = Vec::from(SSE_DELTA);
        expected.extend_from_slice(SSE_END);
        assert_eq!(rest.as_ref(), expected);

        shutdown.send(()).ok();
        handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tee_sees_every_sse_frame() {
        let first_sent = Arc::new(Notify::new());
        let send_rest = Arc::new(Notify::new());
        let stub = sse_stub(first_sent.clone(), send_rest.clone()).await;
        let frames = Arc::new(Mutex::new(0usize));
        let (proxy_addr, shutdown, handle, _) = spawn_proxy_cfg(stub, {
            let frames = frames.clone();
            move |p| p.with_meter(Meter::Count(frames))
        })
        .await;

        send_rest.notify_one();
        let collected = send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            HeaderMap::new(),
            Bytes::from_static(br#"{"stream":true}"#),
        )
        .await;
        assert_eq!(collected.status, StatusCode::OK);
        let mut expected = Vec::from(SSE_START);
        expected.extend_from_slice(SSE_DELTA);
        expected.extend_from_slice(SSE_END);
        assert_eq!(collected.body.as_ref(), expected);

        tokio::task::yield_now().await;
        shutdown.send(()).ok();
        handle.await.unwrap();
        assert_eq!(*frames.lock().unwrap(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kill_the_meter_still_delivers_the_full_response() {
        let first_sent = Arc::new(Notify::new());
        let send_rest = Arc::new(Notify::new());
        let stub = sse_stub(first_sent.clone(), send_rest.clone()).await;
        let (proxy_addr, shutdown, handle, _) =
            spawn_proxy_cfg(stub, |p| p.with_meter(Meter::KillAfter(1))).await;

        let client = Client::builder(TokioExecutor::new()).build_http();
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("http://{proxy_addr}/v1/messages"))
            .body(Full::new(Bytes::from_static(br#"{"stream":true}"#)))
            .unwrap();
        let resp = client.request(req).await.expect("proxy request");
        let mut body = resp.into_body();

        tokio::time::timeout(Duration::from_secs(2), first_sent.notified())
            .await
            .expect("first frame teed (and meter killed)");
        let first = body
            .frame()
            .await
            .expect("frame")
            .expect("ok")
            .into_data()
            .expect("data");
        assert_eq!(first.as_ref(), SSE_START);

        send_rest.notify_one();
        let rest = body
            .collect()
            .await
            .expect("client still got the rest")
            .to_bytes();
        let mut expected = Vec::from(SSE_DELTA);
        expected.extend_from_slice(SSE_END);
        assert_eq!(rest.as_ref(), expected);

        shutdown.send(()).ok();
        handle.await.unwrap();
    }

    fn claude_headers(session: &str, agent: Option<&str>, parent: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            "x-claude-code-session-id",
            HeaderValue::from_str(session).unwrap(),
        );
        if let Some(a) = agent {
            h.insert("x-claude-code-agent-id", HeaderValue::from_str(a).unwrap());
        }
        if let Some(p) = parent {
            h.insert(
                "x-claude-code-parent-agent-id",
                HeaderValue::from_str(p).unwrap(),
            );
        }
        h
    }

    fn tree_walk(proxy: &Proxy, root: &str) -> Vec<(String, u8)> {
        let arena = proxy.arena().lock().unwrap();
        arena
            .get(root)
            .map(|t| {
                t.walk()
                    .into_iter()
                    .map(|(id, d)| (id.to_string(), d))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn tree_children(proxy: &Proxy, root: &str, id: &str) -> Vec<String> {
        let arena = proxy.arena().lock().unwrap();
        arena
            .get(root)
            .map(|t| t.children_of(id).into_iter().map(str::to_string).collect())
            .unwrap_or_default()
    }

    async fn wait_shadow(notes: &Arc<Mutex<Vec<ShadowCmp>>>, n: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if notes.lock().unwrap().len() >= n {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("shadow digest should run");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn declared_headers_build_a_subagent_tree() {
        let stub =
            spawn_http_server(
                |_req| async move { Response::new(Full::new(Bytes::from_static(b"{}"))) },
            )
            .await;
        let (proxy_addr, shutdown, handle, proxy) = spawn_proxy_cfg(stub, |p| p).await;

        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            claude_headers("sess-1", None, None),
            Bytes::from_static(br#"{"messages":[{"role":"user","content":"hi"}]}"#),
        )
        .await;
        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            claude_headers("sess-1", Some("agent-7"), None),
            Bytes::from_static(br#"{"messages":[{"role":"user","content":"sub"}]}"#),
        )
        .await;
        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            claude_headers("sess-1", Some("agent-9"), Some("agent-7")),
            Bytes::from_static(br#"{"messages":[{"role":"user","content":"nested"}]}"#),
        )
        .await;

        assert_eq!(
            tree_walk(&proxy, "sess-1"),
            vec![
                ("sess-1".into(), 0),
                ("agent-7".into(), 1),
                ("agent-9".into(), 2),
            ]
        );
        assert_eq!(tree_children(&proxy, "sess-1", "sess-1"), vec!["agent-7"]);
        assert_eq!(tree_children(&proxy, "sess-1", "agent-7"), vec!["agent-9"]);

        shutdown.send(()).ok();
        handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prefix_digest_does_not_merge_live_sessions() {
        let stub =
            spawn_http_server(
                |_req| async move { Response::new(Full::new(Bytes::from_static(b"{}"))) },
            )
            .await;
        let (proxy_addr, shutdown, handle, proxy) = spawn_proxy_cfg(stub, |p| p).await;

        let shared = Bytes::from_static(
            br#"{"messages":[{"role":"user","content":"sys"},{"role":"user","content":"hello"}]}"#,
        );
        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            HeaderMap::new(),
            shared.clone(),
        )
        .await;
        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            HeaderMap::new(),
            shared,
        )
        .await;

        assert_eq!(
            proxy.arena().lock().unwrap().len(),
            2,
            "shadow digest must not attach live nodes"
        );

        wait_shadow(proxy.shadow_notes(), 2).await;
        let notes = proxy.shadow_notes().lock().unwrap().clone();
        assert!(notes.iter().all(|n| !n.declared));
        assert!(notes.iter().all(|n| n.had_digest));
        assert_eq!(
            notes[0].shadow_root, notes[1].shadow_root,
            "shadow index would have merged; live must not"
        );
        assert_ne!(notes[0].live_root, notes[1].live_root);

        shutdown.send(()).ok();
        handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shadow_digest_compared_against_declared_identity() {
        let stub =
            spawn_http_server(
                |_req| async move { Response::new(Full::new(Bytes::from_static(b"{}"))) },
            )
            .await;
        let (proxy_addr, shutdown, handle, proxy) = spawn_proxy_cfg(stub, |p| p).await;

        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            claude_headers("sess-1", None, None),
            Bytes::from_static(
                br#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"}]}"#,
            ),
        )
        .await;
        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            claude_headers("sess-1", None, None),
            Bytes::from_static(
                br#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"},{"role":"user","content":"c"}]}"#,
            ),
        )
        .await;

        wait_shadow(proxy.shadow_notes(), 2).await;
        let notes = proxy.shadow_notes().lock().unwrap().clone();
        assert!(notes.iter().all(|n| n.declared && n.had_digest));
        assert!(notes.iter().all(|n| n.live_root == "sess-1"));
        assert_eq!(tree_walk(&proxy, "sess-1")[0].0, "sess-1");

        shutdown.send(()).ok();
        handle.await.unwrap();
    }

    const ANTHROPIC_SSE: &[u8] = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":6000,\"cache_creation\":{\"ephemeral_5m_input_tokens\":100,\"ephemeral_1h_input_tokens\":900}}}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":250,\"output_tokens_details\":{\"thinking_tokens\":100}}}\n\n";
    const OPENAI_SSE: &[u8] = b"data: {\"usage\":{\"prompt_tokens\":1000,\"completion_tokens\":40,\"prompt_tokens_details\":{\"cached_tokens\":800},\"completion_tokens_details\":{\"reasoning_tokens\":15}}}\n\ndata: [DONE]\n\n";
    const INCOMPLETE_SSE: &[u8] =
        b"event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":9}}}\n\n";

    async fn wait_usage(proxy: &Proxy, root: &str, pred: impl Fn(&Usage, u32) -> bool) -> Usage {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                {
                    let arena = proxy.arena().lock().unwrap();
                    if let Some(t) = arena.get(root) {
                        let u = t.total_usage();
                        if pred(&u, t.incomplete_requests()) {
                            return u;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("meter should finish the node")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn anthropic_usage_is_merged_into_the_arena_node() {
        let stub = spawn_http_server(|_req| async move {
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Full::new(Bytes::from_static(ANTHROPIC_SSE)))
                .unwrap()
        })
        .await;
        let (proxy_addr, shutdown, handle, proxy) = spawn_proxy_cfg(stub, |p| p).await;
        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            claude_headers("sess-1", None, None),
            Bytes::from_static(b"{}"),
        )
        .await;
        let u = wait_usage(&proxy, "sess-1", |u, _| u.output == 250).await;
        assert_eq!(u.input, 12);
        assert_eq!(u.cache_read, 6000);
        assert_eq!(u.cache_write_5m, 100);
        assert_eq!(u.cache_write_1h, 900);
        assert_eq!(u.output, 250);
        assert_eq!(u.reasoning, 100);
        assert_eq!(
            proxy
                .arena()
                .lock()
                .unwrap()
                .get("sess-1")
                .unwrap()
                .incomplete_requests(),
            0
        );
        shutdown.send(()).ok();
        handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn openai_usage_is_merged_into_the_arena_node() {
        let stub = spawn_http_server(|_req| async move {
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Full::new(Bytes::from_static(OPENAI_SSE)))
                .unwrap()
        })
        .await;
        let (proxy_addr, shutdown, handle, proxy) = spawn_proxy_cfg(stub, |p| p).await;
        send(
            proxy_addr,
            Method::POST,
            "/v1/chat/completions",
            HeaderMap::new(),
            Bytes::from_static(b"{}"),
        )
        .await;
        let roots: Vec<String> = {
            // anonymous root name is assigned at begin; wait until any task has output
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let found: Vec<String> = {
                        let arena = proxy.arena().lock().unwrap();
                        arena
                            .roots()
                            .filter(|t| t.total_usage().output == 40)
                            .map(|t| t.root.clone())
                            .collect()
                    };
                    if !found.is_empty() {
                        return found;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("openai usage")
        };
        let u = proxy
            .arena()
            .lock()
            .unwrap()
            .get(&roots[0])
            .unwrap()
            .total_usage();
        assert_eq!(u.input, 200);
        assert_eq!(u.cache_read, 800);
        assert_eq!(u.output, 40);
        assert_eq!(u.reasoning, 15);
        shutdown.send(()).ok();
        handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn incomplete_stream_is_marked_on_the_node() {
        let stub = spawn_http_server(|_req| async move {
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Full::new(Bytes::from_static(INCOMPLETE_SSE)))
                .unwrap()
        })
        .await;
        let (proxy_addr, shutdown, handle, proxy) = spawn_proxy_cfg(stub, |p| p).await;
        send(
            proxy_addr,
            Method::POST,
            "/v1/messages",
            claude_headers("sess-1", None, None),
            Bytes::from_static(b"{}"),
        )
        .await;
        wait_usage(&proxy, "sess-1", |u, inc| u.input == 9 && inc == 1).await;
        shutdown.send(()).ok();
        handle.await.unwrap();
    }
}
