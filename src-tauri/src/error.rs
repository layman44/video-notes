use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn user_cancelled() -> Self {
        Self {
            code: "USER_CANCELLED".into(),
            message: "任务已由用户暂停或取消".into(),
            details: None,
        }
    }

    pub fn model_not_installed(message: impl Into<String>) -> Self {
        Self {
            code: "MODEL_NOT_INSTALLED".into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            code: "OPERATION_FAILED".into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        if s == "USER_CANCELLED" || s.contains("任务已取消") || s.contains("用户已中断") {
            Self::user_cancelled()
        } else if s.starts_with("MODEL_NOT_INSTALLED:") {
            Self::model_not_installed(s.trim_start_matches("MODEL_NOT_INSTALLED:").trim())
        } else if s.starts_with("TRANSLATION_MODEL_NOT_INSTALLED:") {
            Self::model_not_installed(s.trim_start_matches("TRANSLATION_MODEL_NOT_INSTALLED:").trim())
        } else if s.starts_with("SUMMARY_MODEL_NOT_INSTALLED:") {
            Self::model_not_installed(s.trim_start_matches("SUMMARY_MODEL_NOT_INSTALLED:").trim())
        } else if s.starts_with("ALREADY_DOWNLOADING") {
            Self::new("ALREADY_DOWNLOADING", s)
        } else {
            Self::failed(s)
        }
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        Self::from(s.to_string())
    }
}
