use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tailfin_ledger::{
    default_capture_dir, parse_retention, replay, CaptureStore, Ledger, ReplayOpts, StubBatch,
    DEFAULT_RETENTION,
};
use tailfin_proxy::{serve, Config, Proxy};
use tailfin_wire::RateCard;
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(name = "tailfin", about = "Flight recorder for AI agents")]
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
    /// Write a one-line Tailfin-Cost git trailer/note (capture-grade identity only).
    Stamp(StampArgs),
    /// Per-node cost as hunk-shaped rows.
    Blame(BlameArgs),
    /// Report collisions in a LiteLLM/gateway config.
    Doctor(DoctorArgs),
}

#[derive(clap::Args)]
struct RunArgs {
    /// Bind address.
    #[arg(long, env = "TAILFIN_LISTEN", default_value = "127.0.0.1:7171")]
    listen: SocketAddr,
    /// Provider base URL (e.g. https://api.anthropic.com).
    #[arg(long, env = "TAILFIN_UPSTREAM")]
    upstream: String,
    /// JSONL ledger path.
    #[arg(long, env = "TAILFIN_LEDGER", default_value = "tailfin.jsonl")]
    ledger: PathBuf,
    /// Store full request bodies locally (off by default).
    #[arg(long)]
    capture: bool,
    /// How long to keep captured bodies (e.g. 7d, 24h).
    #[arg(long, default_value = "7d")]
    retention: String,
    /// Directory for captured bodies. Default: tailfin-capture next to the ledger.
    #[arg(long)]
    capture_dir: Option<PathBuf>,
    /// Per-task ceiling in dollars. Requires `--rates`. Honest to within one
    /// in-flight request per branch.
    #[arg(long)]
    max_per_task: Option<f64>,
    /// Fraction of a parent's remaining ceiling minted to each new subagent (e.g. 30%).
    #[arg(long)]
    subagent_share: Option<String>,
    /// TOML rate card (µ$ per token). Without it, reports are token-only.
    #[arg(long)]
    rates: Option<PathBuf>,
}

#[derive(clap::Args)]
struct ReplayArgs {
    /// Max captured tasks to resubmit.
    #[arg(long, default_value_t = 20)]
    sample: usize,
    /// Comma-separated model ids to resubmit against.
    #[arg(long, default_value = "haiku")]
    models: String,
    /// Only tasks newer than this window (e.g. 7d).
    #[arg(long)]
    since: Option<String>,
    /// JSONL ledger path (used to locate the default capture dir).
    #[arg(long, env = "TAILFIN_LEDGER", default_value = "tailfin.jsonl")]
    ledger: PathBuf,
    /// Directory of captured request bodies.
    #[arg(long)]
    capture_dir: Option<PathBuf>,
    /// Force the in-process stub batch (no provider, not the interactive proxy).
    #[arg(long)]
    stub: bool,
}

#[derive(clap::Args)]
struct StampArgs {
    /// Git ref to note (default HEAD). Printed if git notes fails.
    #[arg(default_value = "HEAD")]
    git_ref: String,
    /// JSONL ledger path.
    #[arg(long, env = "TAILFIN_LEDGER", default_value = "tailfin.jsonl")]
    ledger: PathBuf,
    /// TOML rate card (µ$ per token). Without it, the stamp is token-only.
    #[arg(long)]
    rates: Option<PathBuf>,
}

#[derive(clap::Args)]
struct BlameArgs {
    /// JSONL ledger path.
    #[arg(long, env = "TAILFIN_LEDGER", default_value = "tailfin.jsonl")]
    ledger: PathBuf,
    /// TOML rate card (µ$ per token). Without it, blame is token-only.
    #[arg(long)]
    rates: Option<PathBuf>,
}

#[derive(clap::Args)]
struct DoctorArgs {
    /// Path to a LiteLLM / gateway config (YAML/TOML/JSON text).
    config: PathBuf,
}

#[derive(clap::Args)]
struct ReportArgs {
    /// JSONL ledger path.
    #[arg(long, env = "TAILFIN_LEDGER", default_value = "tailfin.jsonl")]
    ledger: PathBuf,
    /// TOML rate card (µ$ per token). Without it, the report is token-only.
    #[arg(long)]
    rates: Option<PathBuf>,
    /// Paste-ready table: no paths, no session or node ids.
    #[arg(long)]
    share: bool,
}

#[tokio::main]
async fn main() {
    if let Err(e) = go().await {
        eprintln!("tailfin: {e}");
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
        Cmd::Stamp(args) => {
            stamp_cmd(args)?;
            Ok(())
        }
        Cmd::Blame(args) => {
            blame_cmd(args)?;
            Ok(())
        }
        Cmd::Doctor(args) => {
            doctor_cmd(args)?;
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
    tailfin_proxy::init_log();
    let ledger = Ledger::open(&args.ledger)?;
    let mut proxy = Proxy::new(cfg.upstream.clone())?.with_ledger(Arc::new(ledger));
    if let Some(dollars) = args.max_per_task {
        let path = args
            .rates
            .as_ref()
            .ok_or("--max-per-task requires --rates")?;
        let rates = load_rates(path)?;
        let micros = (dollars * 1_000_000.0).max(0.0) as u64;
        let share = match args.subagent_share.as_deref() {
            Some(s) => Some(parse_share(s)?),
            None => None,
        };
        eprintln!(
            "tailfin ceiling ${dollars:.4} ({} µ$) share {:?}",
            micros, share
        );
        proxy = proxy.with_budget(micros, rates, share);
    }
    if args.capture {
        let retention = parse_retention(&args.retention).unwrap_or(DEFAULT_RETENTION);
        let dir = args
            .capture_dir
            .unwrap_or_else(|| default_capture_dir(&args.ledger));
        let store = CaptureStore::open(&dir, retention)?;
        let pruned = store.prune().unwrap_or(0);
        eprintln!(
            "tailfin capture on → {} (retention {}, pruned {pruned})",
            dir.display(),
            args.retention
        );
        proxy = proxy.with_capture(Arc::new(store));
    }
    let listener = TcpListener::bind(cfg.listen).await?;
    let bound = listener.local_addr()?;
    eprintln!(
        "tailfin listening on http://{bound} → {} (ledger {})",
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
        tailfin_ledger::render(&records, rates.as_ref(), args.share)
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
        eprintln!("tailfin replay: no ANTHROPIC_API_KEY; stub batch (not a live week of tasks)");
    } else {
        eprintln!("tailfin replay: live batch not wired; stub batch (calendar gate stays open)");
    }
    let rows = replay(&recs, &opts, &StubBatch::default());
    print!("{}", tailfin_ledger::render_table(&rows));
    Ok(())
}

fn stamp_cmd(args: StampArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let records = Ledger::read_all(&args.ledger)?;
    let rates = match args.rates {
        Some(p) => Some(load_rates(&p)?),
        None => None,
    };
    let line = tailfin_ledger::format_stamp(&records, rates.as_ref())?;
    println!("{line}");
    let git = std::process::Command::new("git")
        .args(["notes", "add", "-f", "-m", &line, &args.git_ref])
        .status();
    match git {
        Ok(s) if s.success() => eprintln!("tailfin stamp: noted {}", args.git_ref),
        _ => eprintln!("tailfin stamp: printed only (git notes unavailable)"),
    }
    Ok(())
}

fn blame_cmd(args: BlameArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let records = Ledger::read_all(&args.ledger)?;
    let rates = match args.rates {
        Some(p) => Some(load_rates(&p)?),
        None => None,
    };
    print!("{}", tailfin_ledger::format_blame(&records, rates.as_ref()));
    Ok(())
}

fn doctor_cmd(args: DoctorArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let raw = std::fs::read_to_string(&args.config)?;
    print!(
        "{}",
        tailfin_ledger::render_doctor(&tailfin_ledger::diagnose(&raw))
    );
    Ok(())
}

fn parse_share(s: &str) -> Result<f64, String> {
    let t = s.trim().trim_end_matches('%').trim();
    let v: f64 = t.parse().map_err(|_| format!("bad --subagent-share {s}"))?;
    if s.contains('%') || v > 1.0 {
        Ok((v / 100.0).clamp(0.0, 1.0))
    } else {
        Ok(v.clamp(0.0, 1.0))
    }
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
        let cli =
            Cli::try_parse_from(["tailfin", "report", "--share", "--ledger", "x.jsonl"]).unwrap();
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
        let cli = Cli::try_parse_from(["tailfin", "report"]).unwrap();
        match cli.cmd {
            Cmd::Report(a) => assert!(!a.share),
            _ => panic!("expected report"),
        }
    }

    #[test]
    fn run_capture_defaults_off() {
        let cli =
            Cli::try_parse_from(["tailfin", "run", "--upstream", "https://api.anthropic.com"])
                .unwrap();
        match cli.cmd {
            Cmd::Run(a) => assert!(!a.capture),
            _ => panic!("expected run"),
        }
    }

    #[test]
    fn run_max_per_task_parses() {
        let cli = Cli::try_parse_from([
            "tailfin",
            "run",
            "--upstream",
            "https://api.anthropic.com",
            "--max-per-task",
            "5",
            "--subagent-share",
            "30%",
            "--rates",
            "rates.toml",
        ])
        .unwrap();
        match cli.cmd {
            Cmd::Run(a) => {
                assert_eq!(a.max_per_task, Some(5.0));
                assert_eq!(a.subagent_share.as_deref(), Some("30%"));
            }
            _ => panic!("expected run"),
        }
    }

    #[test]
    fn parse_share_accepts_percent_and_fraction() {
        assert!((parse_share("30%").unwrap() - 0.3).abs() < 1e-9);
        assert!((parse_share("0.3").unwrap() - 0.3).abs() < 1e-9);
        assert!((parse_share("30").unwrap() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn doctor_parses() {
        let cli = Cli::try_parse_from(["tailfin", "doctor", "cfg.yaml"]).unwrap();
        match cli.cmd {
            Cmd::Doctor(a) => assert_eq!(a.config, PathBuf::from("cfg.yaml")),
            _ => panic!("doctor"),
        }
    }

    #[test]
    fn stamp_and_blame_parse() {
        let s = Cli::try_parse_from(["tailfin", "stamp", "HEAD", "--ledger", "x.jsonl"]).unwrap();
        match s.cmd {
            Cmd::Stamp(a) => assert_eq!(a.git_ref, "HEAD"),
            _ => panic!("stamp"),
        }
        let b = Cli::try_parse_from(["tailfin", "blame", "--ledger", "x.jsonl"]).unwrap();
        match b.cmd {
            Cmd::Blame(a) => assert_eq!(a.ledger, PathBuf::from("x.jsonl")),
            _ => panic!("blame"),
        }
    }

    #[test]
    fn replay_flag_parses_sample_and_models() {
        let cli = Cli::try_parse_from([
            "tailfin",
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
