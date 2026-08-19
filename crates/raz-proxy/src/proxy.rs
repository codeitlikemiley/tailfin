use crate::hop::strip_hop_by_hop;
use crate::identity::{messages_from_body, HeaderView};
use crate::tee::TeeBody;
use crate::Error;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, CONTENT_LENGTH, HOST};
use hyper::http::uri::PathAndQuery;
use hyper::{Request, Response, StatusCode, Uri};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use raz_ident::{PrefixDigest, SessionIndex};
use raz_tree::Arena;
use raz_wire::{Dialect, Meter as UsageMeter, SseDecoder, Usage};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type RelayBody = BoxBody<Bytes, BoxError>;

const TEE_CAPACITY: usize = 64;

/// How teed frames are consumed. Default logs counts; tests inject others.
#[derive(Clone, Default)]
pub(crate) enum Meter {
    #[default]
    Log,
    #[cfg(test)]
    Count(std::sync::Arc<std::sync::Mutex<usize>>),
    #[cfg(test)]
    KillAfter(usize),
}

/// One shadow-mode digest observation. Never used to attach a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShadowCmp {
    pub live_root: String,
    pub shadow_root: String,
    pub declared: bool,
    pub had_digest: bool,
}

/// Reverse-proxy that streams request and response bodies through unchanged.
#[derive(Clone)]
pub struct Proxy {
    upstream: Uri,
    client: Client<HttpsConnector<HttpConnector>, BoxBody<Bytes, BoxError>>,
    #[cfg_attr(not(test), allow(dead_code))]
    meter: Meter,
    ident: Arc<Mutex<SessionIndex>>,
    /// Separate index so digest matching cannot attach live nodes (M8 will).
    shadow: Arc<Mutex<SessionIndex>>,
    arena: Arc<Mutex<Arena>>,
    shadow_notes: Arc<Mutex<Vec<ShadowCmp>>>,
}

impl Proxy {
    pub fn new(upstream: Uri) -> Result<Self, Error> {
        install_crypto_provider();
        match upstream.scheme_str() {
            Some("http") | Some("https") => {}
            _ => return Err(Error::InvalidUpstream("scheme must be http or https")),
        }
        if upstream.authority().is_none() {
            return Err(Error::InvalidUpstream("missing host"));
        }
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .map_err(|e| Error::Config(format!("loading native TLS roots: {e}")))?
            .https_or_http()
            .enable_http1()
            .build();
        let client = Client::builder(TokioExecutor::new())
            .http1_preserve_header_case(true)
            .build(https);
        Ok(Self {
            upstream,
            client,
            meter: Meter::Log,
            ident: Arc::new(Mutex::new(SessionIndex::new())),
            shadow: Arc::new(Mutex::new(SessionIndex::new())),
            arena: Arc::new(Mutex::new(Arena::new())),
            shadow_notes: Arc::new(Mutex::new(Vec::new())),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_meter(mut self, meter: Meter) -> Self {
        self.meter = meter;
        self
    }

    #[cfg(test)]
    pub(crate) fn arena(&self) -> &Arc<Mutex<Arena>> {
        &self.arena
    }

    #[cfg(test)]
    pub(crate) fn shadow_notes(&self) -> &Arc<Mutex<Vec<ShadowCmp>>> {
        &self.shadow_notes
    }

    pub fn upstream(&self) -> &Uri {
        &self.upstream
    }

    /// Relay one request. Upstream failures become 502; the body is never
    /// collected on either side of a successful hop.
    pub async fn relay(&self, req: Request<Incoming>) -> Response<RelayBody> {
        match self.relay_inner(req).await {
            Ok(resp) => resp,
            Err(err) => {
                crate::log::log(format!("raz: upstream error: {err}"));
                gateway_error()
            }
        }
    }

    async fn relay_inner(&self, req: Request<Incoming>) -> Result<Response<RelayBody>, BoxError> {
        // Claude Code probes the configured base URL. Do not forward this to Anthropic.
        if req.uri().path() == "/api/hello" {
            return Ok(hello_ok());
        }
        let (mut parts, body) = req.into_parts();
        // Digest is passed as None: live identity is declared headers or
        // anonymous. Prefix matching stays shadow until M8.
        let node = self
            .ident
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resolve(&HeaderView(&parts.headers), None);
        {
            let mut arena = self.arena.lock().unwrap_or_else(|e| e.into_inner());
            arena.task_mut(&node.root).begin(&node, None);
            crate::log::log(format!(
                "raz: begin root={} node={} parent={:?} declared={} conf={:.2} path={}",
                node.root,
                node.node,
                node.parent,
                node.source.is_declared(),
                node.source.confidence(),
                parts.uri.path()
            ));
        }

        // Unknown paths still get the Anthropic meter: Claude Code has used
        // more than `/v1/messages` (sessions event streams, trailing slashes).
        // OpenAI is matched first inside for_path. A miss yields zeros today.
        let dialect = Dialect::for_path(parts.uri.path()).unwrap_or(Dialect::AnthropicMessages);
        let (dtx, drx) = mpsc::channel(TEE_CAPACITY);
        self.spawn_shadow(drx, node.clone(), HeaderViewOwned::from(&parts.headers));

        parts.uri = rewrite_uri(&self.upstream, &parts.uri)?;
        strip_hop_by_hop(&mut parts.headers);
        // So the tee sees plaintext SSE. We never decompress the relayed body.
        parts.headers.remove("accept-encoding");
        if let Some(auth) = parts.uri.authority() {
            parts
                .headers
                .insert(HOST, HeaderValue::from_str(auth.as_str())?);
        }
        // TeeBody is a new hop: leftover Content-Length makes the peer wait
        // forever for bytes that never come (Claude Code freeze after one reply).
        parts.headers.remove(CONTENT_LENGTH);
        let req_body = TeeBody::new(body, dtx)
            .map_err(|e| Box::new(e) as BoxError)
            .boxed();
        let upstream_req = Request::from_parts(parts, req_body);

        let resp = match self.client.request(upstream_req).await {
            Ok(resp) => resp,
            Err(err) => {
                self.finish_node(&node, false);
                return Err(Box::new(err));
            }
        };
        let (mut parts, body) = resp.into_parts();
        strip_hop_by_hop(&mut parts.headers);
        parts.headers.remove(CONTENT_LENGTH);
        let (tx, rx) = mpsc::channel(TEE_CAPACITY);
        self.spawn_meter(rx, Some(dialect), node.clone());
        let body = TeeBody::new(body, tx)
            .map_err(|e| Box::new(e) as BoxError)
            .boxed();
        Ok(Response::from_parts(parts, body))
    }

    fn finish_node(&self, node: &raz_ident::NodeRef, complete: bool) {
        finish_locked(&self.arena, node, &Usage::default(), complete);
    }

    fn spawn_shadow(
        &self,
        mut rx: mpsc::Receiver<Bytes>,
        live: raz_ident::NodeRef,
        headers: HeaderViewOwned,
    ) {
        let shadow = self.shadow.clone();
        let notes = self.shadow_notes.clone();
        tokio::spawn(async move {
            let mut buf = Vec::new();
            while let Some(chunk) = rx.recv().await {
                buf.extend_from_slice(&chunk);
            }
            let digest = messages_from_body(&buf).map(|m| PrefixDigest::from_messages(&m));
            let shadow_node = shadow
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .resolve(&headers, digest);
            let cmp = ShadowCmp {
                live_root: live.root.clone(),
                shadow_root: shadow_node.root.clone(),
                declared: live.source.is_declared(),
                had_digest: digest.is_some(),
            };
            crate::log::log(format!(
                "raz: shadow live_root={} shadow_root={} declared={} digest={} conf={:.2}",
                cmp.live_root,
                cmp.shadow_root,
                cmp.declared,
                cmp.had_digest,
                shadow_node.source.confidence()
            ));
            let mut notes = notes.lock().unwrap_or_else(|e| e.into_inner());
            notes.push(cmp);
            let extra = notes.len().saturating_sub(64);
            if extra > 0 {
                notes.drain(..extra);
            }
        });
    }

    fn spawn_meter(
        &self,
        mut rx: mpsc::Receiver<Bytes>,
        dialect: Option<Dialect>,
        node: raz_ident::NodeRef,
    ) {
        let arena = self.arena.clone();
        #[cfg(test)]
        let kind = self.meter.clone();
        tokio::spawn(async move {
            let mut decoder = SseDecoder::new();
            let mut meter = dialect.map(UsageMeter::new);
            let mut frames = 0usize;
            let mut bytes = 0usize;
            while let Some(chunk) = rx.recv().await {
                frames += 1;
                bytes += chunk.len();
                #[cfg(test)]
                match &kind {
                    Meter::Count(n) => *n.lock().expect("count") += 1,
                    Meter::KillAfter(limit) if frames >= *limit => return,
                    _ => {}
                }
                if let Some(m) = meter.as_mut() {
                    for frame in decoder.push(&chunk) {
                        m.observe(&frame);
                    }
                }
            }
            let (usage, complete) = match meter {
                Some(m) => (m.usage(), m.is_complete()),
                None => (Usage::default(), false),
            };
            if frames > 0 {
                crate::log::log(format!(
                    "raz: teed {frames} frames / {bytes} bytes in={} out={} cache_read={} cache_5m={} cache_1h={} complete={complete}",
                    usage.input, usage.output, usage.cache_read, usage.cache_write_5m, usage.cache_write_1h
                ));
            }
            finish_locked(&arena, &node, &usage, complete);
        });
    }
}

fn finish_locked(arena: &Mutex<Arena>, node: &raz_ident::NodeRef, usage: &Usage, complete: bool) {
    let mut arena = arena.lock().unwrap_or_else(|e| e.into_inner());
    arena.task_mut(&node.root).finish(node, usage, complete);
    if let Some(task) = arena.get(&node.root) {
        let walk: Vec<_> = task
            .walk()
            .into_iter()
            .map(|(id, depth)| format!("{id}:{depth}"))
            .collect();
        crate::log::log(format!("raz: tree {} [{}]", node.root, walk.join(" ")));
    }
}

/// Owned header map so the shadow task can resolve without borrowing the request.
struct HeaderViewOwned {
    inner: hyper::HeaderMap,
}

impl HeaderViewOwned {
    fn from(h: &hyper::HeaderMap) -> Self {
        Self { inner: h.clone() }
    }
}

impl raz_ident::Headers for HeaderViewOwned {
    fn get(&self, name: &str) -> Option<&str> {
        self.inner.get(name).and_then(|v| v.to_str().ok())
    }
}

fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn rewrite_uri(upstream: &Uri, req_uri: &Uri) -> Result<Uri, BoxError> {
    let mut parts = upstream.clone().into_parts();
    parts.path_and_query = Some(
        req_uri
            .path_and_query()
            .cloned()
            .unwrap_or_else(|| PathAndQuery::from_static("/")),
    );
    Ok(Uri::from_parts(parts)?)
}

fn hello_ok() -> Response<RelayBody> {
    let mut resp = Response::new(
        Full::new(Bytes::from_static(br#"{"status":"ok"}"#))
            .map_err(|never: Infallible| -> BoxError { match never {} })
            .boxed(),
    );
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}

fn gateway_error() -> Response<RelayBody> {
    let mut resp = Response::new(
        Full::new(Bytes::from_static(b"bad gateway\n"))
            .map_err(|never: Infallible| -> BoxError { match never {} })
            .boxed(),
    );
    *resp.status_mut() = StatusCode::BAD_GATEWAY;
    resp
}
