use raz_proxy::{serve, Config, Error, Proxy};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        if matches!(&e, Error::Config(m) if m == "help") {
            eprint!("{USAGE}");
            return;
        }
        eprintln!("raz-proxy: {e}");
        std::process::exit(1);
    }
}

const USAGE: &str = "\
raz-proxy — HTTP relay

  --listen ADDR     bind address (default 127.0.0.1:7171, or RAZ_LISTEN)
  --upstream URL    upstream base URL (required, or RAZ_UPSTREAM)
";

async fn run() -> Result<(), Error> {
    let cfg = Config::parse(
        std::env::args().skip(1),
        std::env::var("RAZ_LISTEN").ok().as_deref(),
        std::env::var("RAZ_UPSTREAM").ok().as_deref(),
    )?;
    raz_proxy::init_log();
    let proxy = Proxy::new(cfg.upstream.clone())?;
    let listener = TcpListener::bind(cfg.listen).await?;
    let bound = listener.local_addr()?;
    eprintln!("raz listening on http://{bound} → {}", cfg.upstream);
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
                    eprintln!("raz: installing SIGTERM handler failed: {e}");
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
