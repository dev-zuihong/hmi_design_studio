use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code}: {message}")]
pub struct CoreError {
    pub code: String,
    pub message: String,
}

impl CoreError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn cancelled() -> Self {
        Self::new("CANCELLED", "操作已取消")
    }
}

impl From<std::io::Error> for CoreError {
    fn from(error: std::io::Error) -> Self {
        Self::new("IO", error.to_string())
    }
}
impl From<serde_json::Error> for CoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("JSON", error.to_string())
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
