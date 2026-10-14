use chrono::Utc;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
pub struct AuditEntry {
    pub ts: String,
    pub operation: String,
    pub profile_id: String,
    pub device: String,
    pub address: u64,
    pub length: u64,
    pub segment_id: Option<String>,
    pub outcome: String,
    pub detail: String,
}

pub struct FlashAuditLog {
    path: PathBuf,
    inner: Mutex<()>,
}

impl FlashAuditLog {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            inner: Mutex::new(()),
        }
    }

    pub fn default_path() -> PathBuf {
        PathBuf::from("wideband-flash-audit.jsonl")
    }

    pub fn append(&self, entry: AuditEntry) -> Result<(), String> {
        let line = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
        let _guard = self.inner.lock().map_err(|_| "audit log lock poisoned".to_string())?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("audit log open failed: {}", e))?;
        writeln!(file, "{}", line).map_err(|e| format!("audit log write failed: {}", e))?;
        Ok(())
    }

    pub fn record_write(
        &self,
        operation: &str,
        profile_id: &str,
        device: &str,
        address: u64,
        length: u64,
        segment_id: Option<&str>,
        outcome: &str,
        detail: &str,
    ) -> Result<(), String> {
        self.append(AuditEntry {
            ts: Utc::now().to_rfc3339(),
            operation: operation.to_string(),
            profile_id: profile_id.to_string(),
            device: device.to_string(),
            address,
            length,
            segment_id: segment_id.map(|s| s.to_string()),
            outcome: outcome.to_string(),
            detail: detail.to_string(),
        })
    }
}
