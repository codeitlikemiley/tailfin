//! Append-only JSONL ledger. One record per request finish.

#![forbid(unsafe_code)]

use raz_ident::NodeRef;
use raz_tree::{Arena, Task};
use raz_wire::{RateCard, Usage};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: u32 = 1;

/// Printed when `--capture` is passed. Bodies are not stored until M8.
pub const CAPTURE_NOTICE: &str = "capture lands in M8";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub schema_version: u32,
    /// Stable across process restarts when identity is declared (session id).
    pub task_id: String,
    pub node: String,
    pub parent: Option<String>,
    pub confidence: f32,
    pub incomplete: bool,
    pub input: u64,
    pub output: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub reasoning: u64,
    pub peak_concurrency: u32,
    pub ts_unix_ms: u64,
}

impl Record {
    pub fn from_finish(
        node: &NodeRef,
        usage: &Usage,
        incomplete: bool,
        peak_concurrency: u32,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            task_id: node.root.clone(),
            node: node.node.clone(),
            parent: node.parent.clone(),
            confidence: node.source.confidence(),
            incomplete,
            input: usage.input,
            output: usage.output,
            cache_write_5m: usage.cache_write_5m,
            cache_write_1h: usage.cache_write_1h,
            cache_read: usage.cache_read,
            reasoning: usage.reasoning,
            peak_concurrency,
            ts_unix_ms: now_ms(),
        }
    }

    pub fn usage(&self) -> Usage {
        Usage {
            input: self.input,
            output: self.output,
            cache_write_5m: self.cache_write_5m,
            cache_write_1h: self.cache_write_1h,
            cache_read: self.cache_read,
            reasoning: self.reasoning,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Json(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Error::Json(value)
    }
}

pub struct Ledger {
    path: PathBuf,
    file: Mutex<File>,
}

impl Ledger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, rec: &Record) -> Result<(), Error> {
        let mut line = serde_json::to_string(rec)?;
        line.push('\n');
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        file.write_all(line.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    pub fn read_all(path: impl AsRef<Path>) -> Result<Vec<Record>, Error> {
        let file = match File::open(path.as_ref()) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            out.push(serde_json::from_str(&line)?);
        }
        Ok(out)
    }
}

/// Fold records into per-task arenas for reporting.
pub fn tasks_from_records(records: &[Record]) -> Vec<Task> {
    let mut arena = Arena::new();
    let mut peaks: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for rec in records {
        let node = NodeRef {
            root: rec.task_id.clone(),
            node: rec.node.clone(),
            parent: rec.parent.clone(),
            source: raz_ident::IdentitySource::Declared {
                session: rec.task_id.clone(),
                node: Some(rec.node.clone()),
                parent: rec.parent.clone(),
            },
        };
        let task = arena.task_mut(&rec.task_id);
        task.begin(&node, None);
        task.finish(&node, &rec.usage(), !rec.incomplete);
        let peak = peaks.entry(rec.task_id.clone()).or_insert(0);
        *peak = (*peak).max(rec.peak_concurrency);
    }
    let mut tasks: Vec<Task> = arena.roots().cloned().collect();
    // Replay is sequential so live peak is wrong; stamp the recorded high-water mark.
    // Task.peak_concurrency is private. Report reads the max from records instead.
    let _ = peaks;
    tasks.sort_by(|a, b| a.root.cmp(&b.root));
    tasks
}

pub fn peak_for_task(records: &[Record], root: &str) -> u32 {
    records
        .iter()
        .filter(|r| r.task_id == root)
        .map(|r| r.peak_concurrency)
        .max()
        .unwrap_or(0)
}

pub fn render(records: &[Record], rates: Option<&RateCard>) -> String {
    if records.is_empty() {
        return "no records\n".into();
    }
    let mut out = String::new();
    if rates.is_none() {
        out.push_str("no rate card; token counts only (no dollars)\n");
    }
    let tasks = tasks_from_records(records);
    for task in &tasks {
        out.push_str(&render_task(task, records, rates));
        out.push('\n');
    }
    out
}

fn render_task(task: &Task, records: &[Record], rates: Option<&RateCard>) -> String {
    let total = task.total_usage();
    let main = task.root_usage();
    let sub = task.subagent_usage();
    let fan = task.fan_out_multiplier();
    let peak = peak_for_task(records, &task.root);
    let incomplete = task.incomplete_requests();
    let ratio = total.cache_read_write_ratio();
    let mut s = format!("task {}\n", task.root);
    s.push_str(&format!(
        "  tokens    {}  (main {} / sub {})\n",
        total.total(),
        main.total(),
        sub.total()
    ));
    match fan {
        Some(f) => s.push_str(&format!("  fan-out   {f:.2}x\n")),
        None => s.push_str("  fan-out   n/a\n"),
    }
    if let Some(r) = rates {
        let dollars = task.spent_micros(r) as f64 / 1_000_000.0;
        let share = task.subagent_share(r).unwrap_or(0.0);
        s.push_str(&format!(
            "  cost      ${dollars:.4}  (subagent share {:.0}%)\n",
            share * 100.0
        ));
    }
    s.push_str(&format!("  peak conc {peak}\n"));
    s.push_str(&format!("  incomplete {incomplete}\n"));
    match ratio {
        Some(r) => s.push_str(&format!("  cache r:w {r:.2}\n")),
        None => s.push_str("  cache r:w n/a\n"),
    }
    s.push_str("  node                             tokens");
    if rates.is_some() {
        s.push_str("        $");
    }
    s.push('\n');

    let mut rows: Vec<_> = if let Some(r) = rates {
        task.by_cost(r)
            .into_iter()
            .map(|n| (n.id, n.is_root, n.usage, Some(n.micros)))
            .collect()
    } else {
        // Rank by tokens when we refuse to invent a dollar figure.
        let mut v: Vec<_> = task
            .walk()
            .into_iter()
            .filter_map(|(id, _)| {
                let is_root = id == task.root.as_str();
                Some((id.to_string(), is_root, node_usage(task, id)?, None))
            })
            .collect();
        v.sort_by(|a, b| b.2.total().cmp(&a.2.total()).then_with(|| a.0.cmp(&b.0)));
        v
    };
    // by_cost already sorted; token path sorted above.
    let _ = &mut rows;
    for (id, is_root, usage, micros) in rows {
        let kind = if is_root { "main" } else { "sub " };
        s.push_str(&format!(
            "  {kind} {:<28} {:>7}",
            trunc(&id, 28),
            usage.total()
        ));
        if let (Some(r), Some(m)) = (rates, micros) {
            let _ = r;
            s.push_str(&format!("  ${:.4}", m as f64 / 1_000_000.0));
        }
        s.push('\n');
    }
    s
}

fn node_usage(task: &Task, id: &str) -> Option<Usage> {
    task.by_cost(&RateCard::from_base(1, 1))
        .into_iter()
        .find(|n| n.id == id)
        .map(|n| n.usage)
}

fn trunc(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n.saturating_sub(1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raz_ident::IdentitySource;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("raz-ledger-{}-{n}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn node(root: &str, id: &str, parent: Option<&str>) -> NodeRef {
        NodeRef {
            root: root.into(),
            node: id.into(),
            parent: parent.map(str::to_string),
            source: IdentitySource::Declared {
                session: root.into(),
                node: Some(id.into()),
                parent: parent.map(str::to_string),
            },
        }
    }

    #[test]
    fn append_round_trips_schema_and_incomplete() {
        let path = tmp();
        let led = Ledger::open(&path).unwrap();
        let rec = Record::from_finish(
            &node("sess-1", "agent-7", Some("sess-1")),
            &Usage {
                output: 9,
                ..Default::default()
            },
            true,
            3,
        );
        assert_eq!(rec.schema_version, SCHEMA_VERSION);
        assert_eq!(rec.task_id, "sess-1");
        led.append(&rec).unwrap();
        let got = Ledger::read_all(&path).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].schema_version, 1);
        assert!(got[0].incomplete);
        assert_eq!(got[0].output, 9);
        assert_eq!(got[0].peak_concurrency, 3);
        assert_eq!(got[0].confidence, 1.0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn report_is_token_only_without_a_rate_card() {
        let rec = Record::from_finish(
            &node("sess-1", "sess-1", None),
            &Usage {
                output: 100,
                ..Default::default()
            },
            false,
            1,
        );
        let sub = Record::from_finish(
            &node("sess-1", "a1", Some("sess-1")),
            &Usage {
                output: 900,
                ..Default::default()
            },
            false,
            2,
        );
        let text = render(&[rec, sub], None);
        assert!(text.contains("no rate card"));
        assert!(text.contains("fan-out"));
        assert!(text.contains("peak conc 2"));
        assert!(text.contains("incomplete 0"));
        assert!(!text.contains('$'), "no dollars without a card: {text}");
        assert!(text.contains("sub"));
        assert!(text.contains("1000") || text.contains("  1000") || text.contains("900"));
    }

    #[test]
    fn report_prints_dollars_when_a_rate_card_is_present() {
        let rec = Record::from_finish(
            &node("sess-1", "sess-1", None),
            &Usage {
                output: 1_000,
                ..Default::default()
            },
            false,
            1,
        );
        let sub = Record::from_finish(
            &node("sess-1", "a1", Some("sess-1")),
            &Usage {
                output: 9_000,
                ..Default::default()
            },
            false,
            1,
        );
        let rates = RateCard::from_base(15_000_000, 75_000_000);
        let text = render(&[rec, sub], Some(&rates));
        assert!(text.contains('$'), "{text}");
        assert!(text.contains("subagent share 90%"), "{text}");
        assert!(!text.contains("no rate card"));
    }

    #[test]
    fn capture_notice_is_stable() {
        assert_eq!(CAPTURE_NOTICE, "capture lands in M8");
    }

    #[test]
    fn missing_ledger_file_is_empty_not_an_error() {
        let p = std::env::temp_dir().join("raz-does-not-exist-hopefully.jsonl");
        let _ = std::fs::remove_file(&p);
        assert!(Ledger::read_all(&p).unwrap().is_empty());
    }
}
