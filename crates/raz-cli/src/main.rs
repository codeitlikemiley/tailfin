use clap::{Parser, Subcommand};
use raz_ledger::{Ledger, CAPTURE_NOTICE};
use raz_proxy::{serve, Config, Proxy};
use raz_wire::RateCard;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(name = "raz", about = "Flight recorder for AI agents")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the proxy in the foreground.
    Run(RunArgs),
    /// Print a fan-out report from a ledger file.
    Report(ReportArgs),
}

#[derive(clap::Args)]
struct RunArgs {
    #[arg(long, env = "RAZ_LISTEN", default_value = "127.0.0.1:7171")]
    listen: SocketAddr,
    #[arg(long, env = "RAZ_UPSTREAM")]
    upstream: String,
    #[arg(long, env = "RAZ_LEDGER", default_value = "raz.jsonl")]
    ledger: PathBuf,
    /// Reserved. Does not store request bodies.
    #[arg(long)]
    capture: bool,
}

#[derive(clap::Args)]
struct ReportArgs {
    #[arg(long, env = "RAZ_LEDGER", default_value = "raz.jsonl")]
    ledger: PathBuf,
    #[arg(long)]
    rates: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    if let Err(e) = go().await {
        eprintln!("raz: {e}");
        std::process::exit(1);
    }
}

async fn go() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match Cli::parse().cmd {
        Cmd::Run(args) => run(args).await,
        Cmd::Report(args) => {
            report(args)?;
            Ok(())
        }
    }
}

async fn run(args: RunArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if args.capture {
        eprintln!("{CAPTURE_NOTICE}");
    }
    let cfg = Config {
        listen: args.listen,
        upstream: args
            .upstream
            .parse()
            .map_err(|e| format!("upstream: {e}"))?,
    };
    raz_proxy::init_log();
    let ledger = Ledger::open(&args.ledger)?;
    let proxy = Proxy::new(cfg.upstream.clone())?.with_ledger(Arc::new(ledger));
    let listener = TcpListener::bind(cfg.listen).await?;
    let bound = listener.local_addr()?;
    eprintln!(
        "raz listening on http://{bound} → {} (ledger {})",
        cfg.upstream,
        args.ledger.display()
    );
    let _ = std::io::Write::flush(&mut std::io::stderr());
    serve(listener, proxy, shutdown_signal()).await?;
    Ok(())
}

fn report(args: ReportArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let records = Ledger::read_all(&args.ledger)?;
    let rates = match args.rates {
        Some(p) => Some(load_rates(&p)?),
        None => None,
    };
    print!("{}", raz_ledger::render(&records, rates.as_ref()));
    Ok(())
}

fn load_rates(
    path: &std::path::Path,
) -> Result<RateCard, Box<dyn std::error::Error + Send + Sync>> {
    #[derive(serde::Deserialize)]
    struct File {
        input: u64,
        output: u64,
        cache_write_5m: Option<u64>,
        cache_write_1h: Option<u64>,
        cache_read: Option<u64>,
    }
    let raw = std::fs::read_to_string(path)?;
    let f: File = toml::from_str(&raw)?;
    let mut card = RateCard::from_base(f.input, f.output);
    if let Some(v) = f.cache_write_5m {
        card.cache_write_5m = v;
    }
    if let Some(v) = f.cache_write_1h {
        card.cache_write_1h = v;
    }
    if let Some(v) = f.cache_read {
        card.cache_read = v;
    }
    Ok(card)
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => {
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
