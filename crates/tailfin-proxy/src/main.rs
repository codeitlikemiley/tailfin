use tailfin_proxy::{serve, Config, Error, Proxy};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        if matches!(&e, Error::Config(m) if m == "help") {
            eprint!("{USAGE}");
            return;
        }
        eprintln!("tailfin-proxy: {e}");
        std::process::exit(1);
    }
}

const USAGE: &str = "\
tailfin-proxy — HTTP relay

  --listen ADDR     bind address (default 127.0.0.1:7171, or TAILFIN_LISTEN)
  --upstream URL    upstream base URL (required, or TAILFIN_UPSTREAM)
";

async fn run() -> Result<(), Error> {
    let cfg = Config::parse(
        std::env::args().skip(1),
        std::env::var("TAILFIN_LISTEN").ok().as_deref(),
        std::env::var("TAILFIN_UPSTREAM").ok().as_deref(),
    )?;
    tailfin_proxy::init_log();
    let proxy = Proxy::new(cfg.upstream.clone())?;
    let listener = TcpListener::bind(cfg.listen).await?;
    let bound = listener.local_addr()?;
    eprintln!("tailfin listening on http://{bound} → {}", cfg.upstream);
    let _ = std::io::Write::flush(&mut std::io::stderr());
    serve(listener, proxy, shutdown_signal()).await
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("tailfin: installing SIGTERM handler failed: {e}");
                    let _ = ctrl_c.await;
                    return;
                }
            };
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}
