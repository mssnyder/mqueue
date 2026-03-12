#[derive(Debug, thiserror::Error)]
pub enum MqError {
    #[error("IMAP error: {0}")]
    Imap(#[from] async_imap::error::Error),

    #[error("SMTP error: {0}")]
    Smtp(#[from] lettre::transport::smtp::Error),

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("Database error: {0}")]
    Db(String),

    #[error("Email parse error: {0}")]
    Parse(String),

    #[error("Network unavailable")]
    Offline,

    #[error("Token expired and refresh failed")]
    TokenExpired,

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl MqError {
    /// Whether this error is transient and the operation should be retried.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            MqError::Imap(_) | MqError::Smtp(_) | MqError::Offline
        )
    }

    /// Whether this error indicates an authentication failure.
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, MqError::TokenExpired | MqError::OAuth(_))
    }

    /// Human-readable message suitable for displaying in the UI.
    pub fn user_message(&self) -> String {
        match self {
            MqError::Imap(_) => "Failed to communicate with the mail server.".into(),
            MqError::Smtp(_) => "Failed to send the email.".into(),
            MqError::OAuth(msg) => format!("Authentication error: {msg}"),
            MqError::Db(msg) => format!("Database error: {msg}"),
            MqError::Parse(_) => "Failed to parse the email.".into(),
            MqError::Offline => "No internet connection.".into(),
            MqError::TokenExpired => "Your session has expired. Please re-authenticate.".into(),
            MqError::Config(msg) => format!("Configuration error: {msg}"),
            MqError::Other(err) => format!("Unexpected error: {err}"),
        }
    }
}

pub type Result<T> = std::result::Result<T, MqError>;
