use crate::{Error, Proxy};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::future::Future;
use std::pin::pin;
use tokio::net::TcpListener;

/// Accept loop with graceful shutdown: in-flight requests finish, new
/// connections are not accepted after `shutdown` resolves.
///
/// Connections are `with_upgrades()` so Codex's `ws://…/v1/responses` handshake
/// can hijack the socket. hyper-util's GracefulShutdown does not implement
/// `GracefulConnection` for `UpgradeableConnection`, so we await those
/// connections on their own tasks.
pub async fn serve(
    listener: TcpListener,
    proxy: Proxy,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error> {
    let mut shutdown = pin!(shutdown);
    let mut http = http1::Builder::new();
    http.preserve_header_case(true);
    // SSE + keep-alive reuse is how a completed reply leaves the TUI frozen.
    http.keep_alive(false);

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = match result {
                    Ok(s) => s,
                    Err(e) => {
                        crate::log::log(format!("tailfin: accept error: {e}"));
                        continue;
                    }
                };
                let io = TokioIo::new(stream);
                let proxy = proxy.clone();
                let conn = http
                    .serve_connection(
                        io,
                        service_fn(move |req| {
                            let proxy = proxy.clone();
                            async move { Ok::<_, Infallible>(proxy.relay(req).await) }
                        }),
                    )
                    .with_upgrades();
                tokio::spawn(async move {
                    if let Err(e) = conn.await {
                        let msg = e.to_string();
                        // Client abort after the stream is done is normal.
                        if msg.contains("before message completed")
                            || msg.contains("connection reset")
                        {
                            return;
                        }
                        crate::log::log(format!("tailfin: connection error: {e}"));
                    }
                });
            }
            _ = &mut shutdown => {
                break;
            }
        }
    }
    drop(listener);
    Ok(())
}
