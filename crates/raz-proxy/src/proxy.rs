use crate::hop::strip_hop_by_hop;
use crate::tee::TeeBody;
use crate::Error;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, HOST};
use hyper::http::uri::PathAndQuery;
use hyper::{Request, Response, StatusCode, Uri};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::convert::Infallible;
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

/// Reverse-proxy that streams request and response bodies through unchanged.
#[derive(Clone)]
pub struct Proxy {
    upstream: Uri,
    client: Client<HttpsConnector<HttpConnector>, Incoming>,
    meter: Meter,
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
        })
    }

    #[cfg(test)]
    pub(crate) fn with_meter(mut self, meter: Meter) -> Self {
        self.meter = meter;
        self
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
                eprintln!("raz: upstream error: {err}");
                gateway_error()
            }
        }
    }

    async fn relay_inner(&self, req: Request<Incoming>) -> Result<Response<RelayBody>, BoxError> {
        let (mut parts, body) = req.into_parts();
        parts.uri = rewrite_uri(&self.upstream, &parts.uri)?;
        strip_hop_by_hop(&mut parts.headers);
        // Host names this hop's target, not the client's view of the proxy.
        if let Some(auth) = parts.uri.authority() {
            parts
                .headers
                .insert(HOST, HeaderValue::from_str(auth.as_str())?);
        }
        let upstream_req = Request::from_parts(parts, body);
        let resp = self.client.request(upstream_req).await?;
        let (mut parts, body) = resp.into_parts();
        strip_hop_by_hop(&mut parts.headers);
        let (tx, rx) = mpsc::channel(TEE_CAPACITY);
        self.spawn_meter(rx);
        let body = TeeBody::new(body, tx)
            .map_err(|e| Box::new(e) as BoxError)
            .boxed();
        Ok(Response::from_parts(parts, body))
    }

    fn spawn_meter(&self, mut rx: mpsc::Receiver<Bytes>) {
        match self.meter.clone() {
            Meter::Log => {
                tokio::spawn(async move {
                    let mut frames = 0usize;
                    let mut bytes = 0usize;
                    while let Some(chunk) = rx.recv().await {
                        frames += 1;
                        bytes += chunk.len();
                    }
                    if frames > 0 {
                        eprintln!("raz: teed {frames} frames / {bytes} bytes");
                    }
                });
            }
            #[cfg(test)]
            Meter::Count(n) => {
                tokio::spawn(async move {
                    while rx.recv().await.is_some() {
                        *n.lock().expect("count") += 1;
                    }
                });
            }
            #[cfg(test)]
            Meter::KillAfter(limit) => {
                tokio::spawn(async move {
                    for _ in 0..limit {
                        if rx.recv().await.is_none() {
                            return;
                        }
                    }
                    panic!("kill-the-meter");
                });
            }
        }
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

fn gateway_error() -> Response<RelayBody> {
    let mut resp = Response::new(
        Full::new(Bytes::from_static(b"bad gateway\n"))
            .map_err(|never: Infallible| -> BoxError { match never {} })
            .boxed(),
    );
    *resp.status_mut() = StatusCode::BAD_GATEWAY;
    resp
}
