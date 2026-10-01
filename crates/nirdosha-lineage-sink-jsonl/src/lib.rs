//! JSONL LineageSink driver (RFC 0026 §14).

use nirdosha_lineage::LineageObservation;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkError {
    Unconfigured,
    Io(String),
}

pub trait LineageSink: Send + Sync {
    fn emit(&self, obs: &LineageObservation) -> Result<(), SinkError>;
    fn flush(&self) -> Result<(), SinkError>;
}

pub struct JsonlLineageSink {
    path: Option<PathBuf>,
    buffer: Mutex<Vec<u8>>,
}

impl JsonlLineageSink {
    pub fn new(path: Option<impl AsRef<Path>>) -> Self {
        Self {
            path: path.map(|p| p.as_ref().to_path_buf()),
            buffer: Mutex::new(Vec::new()),
        }
    }
}

impl LineageSink for JsonlLineageSink {
    fn emit(&self, obs: &LineageObservation) -> Result<(), SinkError> {
        let path = self.path.as_ref().ok_or(SinkError::Unconfigured)?;
        let content = obs.to_audit_content();
        let json_line = serde_json::to_vec(&content).map_err(|e| SinkError::Io(e.to_string()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| SinkError::Io(e.to_string()))?;
        file.write_all(&json_line).map_err(|e| SinkError::Io(e.to_string()))?;
        file.write_all(b"\n").map_err(|e| SinkError::Io(e.to_string()))?;
        Ok(())
    }

    fn flush(&self) -> Result<(), SinkError> {
        if self.path.is_none() {
            return Err(SinkError::Unconfigured);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_sink_returns_unconfigured() {
        let sink = JsonlLineageSink::new(None::<PathBuf>);
        assert!(sink.flush().is_err());
    }
}
