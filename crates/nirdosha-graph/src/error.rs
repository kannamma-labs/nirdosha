use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub details: Value,
    pub recovery: String,
}

impl Error {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), retryable: matches!(code, "BUSY" | "BACKPRESSURE" | "STORAGE_UNAVAILABLE"), details: json!({}), recovery: "Inspect the error, refresh relevant state, and retry only after resolving the cause.".into() }
    }
    pub fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
    pub fn envelope(&self) -> Value {
        json!({"ok":false,"graph":null,"error":self})
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        let code = match e.sqlite_error_code() {
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => "BUSY",
            _ => "STORAGE_UNAVAILABLE",
        };
        Self::new(code, e.to_string())
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new("STORAGE_UNAVAILABLE", e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("SCHEMA_INVALID", e.to_string())
    }
}
