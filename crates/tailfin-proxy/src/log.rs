use std::sync::OnceLock;
use tokio::sync::mpsc;

static TX: OnceLock<mpsc::Sender<String>> = OnceLock::new();

/// Drain logs on a dedicated task so `eprintln` cannot stall the relay.
pub fn init() {
    if TX.get().is_some() {
        return;
    }
    let (tx, mut rx) = mpsc::channel::<String>(256);
    let _ = TX.set(tx);
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            eprintln!("{line}");
        }
    });
}

pub fn log(msg: impl Into<String>) {
    let msg = msg.into();
    if let Some(tx) = TX.get() {
        let _ = tx.try_send(msg);
    } else {
        eprintln!("{msg}");
    }
}
