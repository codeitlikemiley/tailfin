//! Local request-body capture. Opt-in, never on the response path.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const CAPTURE_SCHEMA: u32 = 1;
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub schema_version: u32,
    pub capture_id: String,
    pub task_id: String,
    pub node: String,
    pub parent: Option<String>,
    pub ts_unix_ms: u64,
    pub method: String,
    pub path: String,
    pub model: Option<String>,
    pub message_count: u32,
    pub tool_calls: u32,
    /// Full request body as UTF-8 (lossy). Local only.
    pub body: String,
}

impl CaptureRecord {
    pub fn shape(&self) -> &'static str {
        let b = self.body.to_ascii_lowercase();
        if b.contains("#[test]") || b.contains("cargo test") {
            "test-generation"
        } else if b.contains("fn main") || b.contains("cargo build") {
            "code-compile"
        } else if self.tool_calls > 0 || b.contains("\"tools\"") {
            "tool-use"
        } else if self.message_count >= 8 {
            "long-context"
        } else {
            "chat"
        }
    }
}

pub fn parse_retention(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let n: u64 = n.parse().map_err(|_| format!("bad retention: {s}"))?;
    match unit {
        "s" => Ok(Duration::from_secs(n)),
        "m" => Ok(Duration::from_secs(n * 60)),
        "h" => Ok(Duration::from_secs(n * 3600)),
        "d" | "" => Ok(Duration::from_secs(n * 86400)),
        _ => Err(format!("bad retention unit in {s}")),
    }
}

pub fn body_meta(body: &str) -> (Option<String>, u32, u32) {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let model = v.get("model").and_then(|m| m.as_str()).map(str::to_string);
    let msgs = v.get("messages").and_then(|m| m.as_array());
    let message_count = msgs.map(|a| a.len() as u32).unwrap_or(0);
    let mut tool_calls = v
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);
    if let Some(msgs) = msgs {
        for m in msgs {
            let s = m.to_string();
            if s.contains("tool_use") || s.contains("tool_result") {
                tool_calls += 1;
            }
        }
    }
    (model, message_count, tool_calls)
}

pub struct CaptureStore {
    dir: PathBuf,
    retention: Duration,
}

impl CaptureStore {
    pub fn open(dir: impl AsRef<Path>, retention: Duration) -> Result<Self, std::io::Error> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir, retention })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save(&self, rec: &CaptureRecord) -> Result<PathBuf, std::io::Error> {
        let sub = self.dir.join(safe_id(&rec.task_id));
        fs::create_dir_all(&sub)?;
        let path = sub.join(format!("{}.json", safe_id(&rec.capture_id)));
        let mut line = serde_json::to_string(rec)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        fs::write(&path, line)?;
        Ok(path)
    }

    pub fn load_all(&self) -> Result<Vec<CaptureRecord>, Box<dyn std::error::Error + Send + Sync>> {
        let mut out = Vec::new();
        if !self.dir.exists() {
            return Ok(out);
        }
        for task in fs::read_dir(&self.dir)? {
            let task = task?;
            if !task.file_type()?.is_dir() {
                continue;
            }
            for f in fs::read_dir(task.path())? {
                let f = f?;
                if f.path().extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let raw = fs::read_to_string(f.path())?;
                let rec: CaptureRecord = serde_json::from_str(raw.trim())?;
                out.push(rec);
            }
        }
        out.sort_by(|a, b| {
            a.ts_unix_ms
                .cmp(&b.ts_unix_ms)
                .then(a.capture_id.cmp(&b.capture_id))
        });
        Ok(out)
    }

    pub fn prune(&self) -> Result<usize, std::io::Error> {
        let cutoff = now_ms().saturating_sub(self.retention.as_millis() as u64);
        let mut n = 0;
        if !self.dir.exists() {
            return Ok(0);
        }
        for task in fs::read_dir(&self.dir)? {
            let task = task?;
            if !task.file_type()?.is_dir() {
                continue;
            }
            for f in fs::read_dir(task.path())? {
                let f = f?;
                let path = f.path();
                let raw = match fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let rec: CaptureRecord = match serde_json::from_str(raw.trim()) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                if rec.ts_unix_ms < cutoff {
                    fs::remove_file(&path)?;
                    n += 1;
                }
            }
            if fs::read_dir(task.path())?.next().is_none() {
                let _ = fs::remove_dir(task.path());
            }
        }
        Ok(n)
    }
}

pub fn default_capture_dir(ledger: &Path) -> PathBuf {
    match ledger.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join("tailfin-capture"),
        _ => PathBuf::from("tailfin-capture"),
    }
}

fn safe_id(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("tailfin-cap-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    fn rec(id: &str, body: &str, ts: u64) -> CaptureRecord {
        let (model, message_count, tool_calls) = body_meta(body);
        CaptureRecord {
            schema_version: CAPTURE_SCHEMA,
            capture_id: id.into(),
            task_id: "sess-1".into(),
            node: "sess-1".into(),
            parent: None,
            ts_unix_ms: ts,
            method: "POST".into(),
            path: "/v1/messages".into(),
            model,
            message_count,
            tool_calls,
            body: body.into(),
        }
    }

    #[test]
    fn save_round_trips_schema_and_body() {
        let dir = tmp();
        let store = CaptureStore::open(&dir, DEFAULT_RETENTION).unwrap();
        let body = r#"{"model":"claude","messages":[{"role":"user","content":"hi"}]}"#;
        store.save(&rec("c1", body, 1000)).unwrap();
        let got = store.load_all().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].schema_version, CAPTURE_SCHEMA);
        assert_eq!(got[0].body, body);
        assert_eq!(got[0].model.as_deref(), Some("claude"));
        assert_eq!(got[0].message_count, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_dir_loads_empty() {
        let p = std::env::temp_dir().join("tailfin-cap-missing-hopefully");
        let _ = fs::remove_dir_all(&p);
        let store = CaptureStore {
            dir: p,
            retention: DEFAULT_RETENTION,
        };
        assert!(store.load_all().unwrap().is_empty());
    }

    #[test]
    fn prune_drops_records_older_than_retention() {
        let dir = tmp();
        let store = CaptureStore::open(&dir, Duration::from_secs(10)).unwrap();
        let now = now_ms();
        store
            .save(&rec("old", "{}", now.saturating_sub(60_000)))
            .unwrap();
        store.save(&rec("new", "{}", now)).unwrap();
        let n = store.prune().unwrap();
        assert_eq!(n, 1);
        let got = store.load_all().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].capture_id, "new");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_retention_units() {
        assert_eq!(
            parse_retention("7d").unwrap(),
            Duration::from_secs(7 * 86400)
        );
        assert_eq!(
            parse_retention("24h").unwrap(),
            Duration::from_secs(24 * 3600)
        );
        assert_eq!(
            parse_retention("30m").unwrap(),
            Duration::from_secs(30 * 60)
        );
    }

    #[test]
    fn path_like_task_ids_do_not_escape_the_store() {
        let dir = tmp();
        let store = CaptureStore::open(&dir, DEFAULT_RETENTION).unwrap();
        let mut r = rec("c1", "{}", 1);
        r.task_id = "/Users/alice/secret".into();
        let path = store.save(&r).unwrap();
        assert!(path.starts_with(&dir));
        let rel = path.strip_prefix(&dir).expect("under store");
        assert!(
            !rel.to_string_lossy().contains(".."),
            "must not walk out: {rel:?}"
        );
        assert_eq!(rel.components().count(), 2, "{rel:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
