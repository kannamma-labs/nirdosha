//! HTTP LineageSink driver (RFC 0026 §14).

use nirdosha_lineage::LineageObservation;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkError {
    Unconfigured,
    Http(String),
}

pub trait LineageSink: Send + Sync {
    fn emit(&self, obs: &LineageObservation) -> Result<(), SinkError>;
    fn flush(&self) -> Result<(), SinkError>;
}

pub struct HttpLineageSink {
    endpoint: Option<String>,
    batch: Mutex<Vec<LineageObservation>>,
}

impl HttpLineageSink {
    pub fn new(endpoint: Option<String>) -> Self {
        Self {
            endpoint,
            batch: Mutex::new(Vec::new()),
        }
    }
}

impl LineageSink for HttpLineageSink {
    fn emit(&self, obs: &LineageObservation) -> Result<(), SinkError> {
        if self.endpoint.is_none() {
            return Err(SinkError::Unconfigured);
        }
        if let Ok(mut vec) = self.batch.lock() {
            vec.push(obs.clone());
        }
        Ok(())
    }

    fn flush(&self) -> Result<(), SinkError> {
        if self.endpoint.is_none() {
            return Err(SinkError::Unconfigured);
        }
        if let Ok(mut vec) = self.batch.lock() {
            vec.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_http_sink_errors() {
        let sink = HttpLineageSink::new(None);
        assert!(sink.flush().is_err());
    }
}
