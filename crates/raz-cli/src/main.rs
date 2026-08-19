use clap::{Parser, Subcommand};
use raz_ledger::{
    default_capture_dir, parse_retention, replay, CaptureStore, Ledger, ReplayOpts, StubBatch,
    DEFAULT_RETENTION,
};
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
    /// Replay captured tasks via a batch sink (never the interactive proxy).
    Replay(ReplayArgs),
}

#[derive(clap::Args)]
struct RunArgs {
    #[arg(long, env = "RAZ_LISTEN", default_value = "127.0.0.1:7171")]
    listen: SocketAddr,
    #[arg(long, env = "RAZ_UPSTREAM")]
    upstream: String,
    #[arg(long, env = "RAZ_LEDGER", default_value = "raz.jsonl")]
    ledger: PathBuf,
    /// Store full request bodies locally (off by default).
    #[arg(long)]
    capture: bool,
    /// How long to keep captured bodies (e.g. 7d, 24h).
    #[arg(long, default_value = "7d")]
    retention: String,
    /// Directory for captured bodies. Default: raz-capture next to the ledger.
    #[arg(long)]
    capture_dir: Option<PathBuf>,
}

#[derive(clap::Args)]
struct ReplayArgs {
    #[arg(long, default_value_t = 20)]
    sample: usize,
    /// Comma-separated model ids to resubmit against.
    #[arg(long, default_value = "haiku")]
    models: String,
    /// Only tasks newer than this window (e.g. 7d).
    #[arg(long)]
    since: Option<String>,
    #[arg(long, env = "RAZ_LEDGER", default_value = "raz.jsonl")]
    ledger: PathBuf,
    #[arg(long)]
    capture_dir: Option<PathBuf>,
    /// Force the in-process stub batch (no provider, not the interactive proxy).
    #[arg(long)]
    stub: bool,
}

#[derive(clap::Args)]
struct ReportArgs {
    #[arg(long, env = "RAZ_LEDGER", default_value = "raz.jsonl")]
    ledger: PathBuf,
    #[arg(long)]
    rates: Option<PathBuf>,
    /// Paste-ready table: no paths, no session or node ids.
    #[arg(long)]
    share: bool,
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
        Cmd::Replay(args) => {
            replay_cmd(args)?;
            Ok(())
        }
    }
}

async fn run(args: RunArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cfg = Config {
        listen: args.listen,
        upstream: args
            .upstream
            .parse()
            .map_err(|e| format!("upstream: {e}"))?,
    };
    raz_proxy::init_log();
    let ledger = Ledger::open(&args.ledger)?;
    let mut proxy = Proxy::new(cfg.upstream.clone())?.with_ledger(Arc::new(ledger));
    if args.capture {
        let retention = parse_retention(&args.retention).unwrap_or(DEFAULT_RETENTION);
        let dir = args
            .capture_dir
            .unwrap_or_else(|| default_capture_dir(&args.ledger));
        let store = CaptureStore::open(&dir, retention)?;
        let pruned = store.prune().unwrap_or(0);
        eprintln!(
            "raz capture on → {} (retention {}, pruned {pruned})",
            dir.display(),
            args.retention
        );
        proxy = proxy.with_capture(Arc::new(store));
    }
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
    print!(
        "{}",
        raz_ledger::render(&records, rates.as_ref(), args.share)
    );
    Ok(())
}

fn replay_cmd(args: ReplayArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dir = args
        .capture_dir
        .unwrap_or_else(|| default_capture_dir(&args.ledger));
    let store = CaptureStore::open(&dir, DEFAULT_RETENTION)?;
    let recs = store.load_all().map_err(|e| e.to_string())?;
    let since_ms = match args.since.as_deref() {
        Some(s) => {
            let d = parse_retention(s)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            Some(now.saturating_sub(d.as_millis() as u64))
        }
        None => None,
    };
    let opts = ReplayOpts {
        sample: args.sample,
        models: args
            .models
            .split(',')
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect(),
        since_ms,
    };
    // Live provider batch APIs need the user's key and are async jobs.
    // Without a key we still run the shipped replay path against StubBatch —
    // never the interactive listener.
    let _ = args.stub;
    if std::env::var_os("ANTHROPIC_API_KEY").is_none() {
        eprintln!("raz replay: no ANTHROPIC_API_KEY; stub batch (not a live week of tasks)");
    } else {
        eprintln!("raz replay: live batch not wired; stub batch (calendar gate stays open)");
    }
    let rows = replay(&recs, &opts, &StubBatch::default());
    print!("{}", raz_ledger::render_table(&rows));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_share_flag_parses() {
        let cli = Cli::try_parse_from(["raz", "report", "--share", "--ledger", "x.jsonl"]).unwrap();
        match cli.cmd {
            Cmd::Report(a) => {
                assert!(a.share);
                assert_eq!(a.ledger, PathBuf::from("x.jsonl"));
            }
            _ => panic!("expected report"),
        }
    }

    #[test]
    fn report_without_share_defaults_off() {
        let cli = Cli::try_parse_from(["raz", "report"]).unwrap();
        match cli.cmd {
            Cmd::Report(a) => assert!(!a.share),
            _ => panic!("expected report"),
        }
    }

    #[test]
    fn run_capture_defaults_off() {
        let cli =
            Cli::try_parse_from(["raz", "run", "--upstream", "https://api.anthropic.com"]).unwrap();
        match cli.cmd {
            Cmd::Run(a) => assert!(!a.capture),
            _ => panic!("expected run"),
        }
    }

    #[test]
    fn replay_flag_parses_sample_and_models() {
        let cli = Cli::try_parse_from([
            "raz",
            "replay",
            "--sample",
            "3",
            "--models",
            "haiku,sonnet",
            "--stub",
        ])
        .unwrap();
        match cli.cmd {
            Cmd::Replay(a) => {
                assert_eq!(a.sample, 3);
                assert_eq!(a.models, "haiku,sonnet");
                assert!(a.stub);
            }
            _ => panic!("expected replay"),
        }
    }
}
