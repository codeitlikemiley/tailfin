use crate::hop::strip_hop_by_hop;
use crate::Error;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, HOST};
use hyper::http::uri::PathAndQuery;
use hyper::{Request, Response, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::convert::Infallible;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type RelayBody = BoxBody<Bytes, BoxError>;

/// Reverse-proxy that streams request and response bodies through unchanged.
#[derive(Clone)]
pub struct Proxy {
    upstream: Uri,
    client: Client<HttpConnector, Incoming>,
}

impl Proxy {
    pub fn new(upstream: Uri) -> Result<Self, Error> {
        match upstream.scheme_str() {
            Some("http") => {}
            Some("https") => {
                return Err(Error::InvalidUpstream(
                    "https upstream needs rustls (not in M1)",
                ));
            }
            _ => return Err(Error::InvalidUpstream("scheme must be http")),
        }
        if upstream.authority().is_none() {
            return Err(Error::InvalidUpstream("missing host"));
        }
        let client = Client::builder(TokioExecutor::new())
            .http1_preserve_header_case(true)
            .build_http();
        Ok(Self { upstream, client })
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
        let body = body.map_err(|e| Box::new(e) as BoxError).boxed();
        Ok(Response::from_parts(parts, body))
    }
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
