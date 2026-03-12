use serde::{Deserialize, Serialize};

/// Represents a configured Gmail account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// Unique identifier (matches DB row id).
    pub id: Option<i64>,
    /// Gmail address (e.g. "user@gmail.com").
    pub email: String,
    /// Display name (e.g. "John Doe").
    pub display_name: Option<String>,
}

impl Account {
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            id: None,
            email: email.into(),
            display_name: None,
        }
    }

    /// The IMAP server for Gmail.
    pub fn imap_host(&self) -> &str {
        "imap.gmail.com"
    }

    /// The IMAP port for Gmail (TLS).
    pub fn imap_port(&self) -> u16 {
        993
    }

    /// The SMTP server for Gmail.
    pub fn smtp_host(&self) -> &str {
        "smtp.gmail.com"
    }

    /// The SMTP port for Gmail (STARTTLS).
    pub fn smtp_port(&self) -> u16 {
        587
    }
}
