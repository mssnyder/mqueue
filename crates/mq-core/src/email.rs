use serde::{Deserialize, Serialize};

/// A parsed email address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

/// Flags on an email message.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageFlags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub deleted: bool,
    pub draft: bool,
}

/// A parsed email message (domain type, not DB row).
#[derive(Debug, Clone)]
pub struct Email {
    /// IMAP UID within its mailbox.
    pub uid: u32,
    /// Gmail-specific message ID (X-GM-MSGID).
    pub gmail_msg_id: Option<u64>,
    /// Gmail-specific thread ID (X-GM-THRID).
    pub gmail_thread_id: Option<u64>,
    /// RFC 5322 Message-ID header.
    pub message_id: Option<String>,
    /// In-Reply-To header.
    pub in_reply_to: Option<String>,
    /// References header values.
    pub references: Vec<String>,

    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: Option<String>,
    pub date: Option<String>,

    /// Short snippet of the body text.
    pub snippet: Option<String>,

    pub flags: MessageFlags,
    /// Gmail labels (X-GM-LABELS).
    pub labels: Vec<String>,

    pub has_attachments: bool,

    /// Raw List-Unsubscribe header.
    pub list_unsubscribe: Option<String>,
    /// Raw List-Unsubscribe-Post header.
    pub list_unsubscribe_post: Option<String>,
}

/// Normalize an RFC 2822 / IMAP ENVELOPE date string to ISO 8601 format
/// for sortable storage (e.g. "2026-02-18T03:35:54+00:00").
///
/// Falls back to the original string if parsing fails.
pub fn normalize_date(date_str: &str) -> String {
    // Try RFC 2822 format first (most common from IMAP ENVELOPE)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(date_str.trim()) {
        return dt.to_rfc3339();
    }
    // Try with common variations: some servers omit the day name
    // e.g. "18 Feb 2026 03:35:54 +0000"
    if let Ok(dt) = chrono::DateTime::parse_from_str(date_str.trim(), "%d %b %Y %H:%M:%S %z") {
        return dt.to_rfc3339();
    }
    // Try without seconds
    if let Ok(dt) = chrono::DateTime::parse_from_str(date_str.trim(), "%a, %d %b %Y %H:%M %z") {
        return dt.to_rfc3339();
    }
    // Some dates have extra stuff like "(UTC)" at the end — strip it
    let cleaned = date_str.trim().trim_end_matches(|c: char| c == ')' || c == '(' || c.is_alphabetic() || c == ' ');
    if cleaned != date_str.trim() {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(cleaned.trim()) {
            return dt.to_rfc3339();
        }
    }
    // Return original if all parsing fails
    date_str.to_string()
}

/// Format an ISO 8601 / RFC 3339 date string for user-facing display.
///
/// Uses the configured time format (12h / 24h). Shows relative labels
/// for today and recent dates.
pub fn format_display_date(date_str: &str) -> String {
    use crate::config::{AppConfig, TimeFormat};
    use chrono::{DateTime, Local, NaiveDateTime};

    let time_format = AppConfig::load()
        .map(|c| c.appearance.time_format)
        .unwrap_or_default();

    let dt = if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
        dt.with_timezone(&Local)
    } else if let Ok(naive) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S") {
        match naive.and_local_timezone(Local).single() {
            Some(dt) => dt,
            None => return date_str.to_string(),
        }
    } else {
        return if date_str.len() > 16 {
            date_str[..16].to_string()
        } else {
            date_str.to_string()
        };
    };

    let now = Local::now();
    let today = now.date_naive();
    let msg_date = dt.date_naive();

    match time_format {
        TimeFormat::TwelveHour => {
            if msg_date == today {
                dt.format("%-I:%M %p").to_string()
            } else if (today - msg_date).num_days() < 7 {
                dt.format("%a %-I:%M %p").to_string()
            } else {
                dt.format("%-m/%d/%Y %-I:%M %p").to_string()
            }
        }
        TimeFormat::TwentyFourHour => {
            if msg_date == today {
                dt.format("%H:%M").to_string()
            } else if (today - msg_date).num_days() < 7 {
                dt.format("%a %H:%M").to_string()
            } else {
                dt.format("%Y-%m-%d %H:%M").to_string()
            }
        }
    }
}

/// Metadata about an attachment.
#[derive(Debug, Clone)]
pub struct Attachment {
    pub filename: Option<String>,
    pub mime_type: String,
    pub size: Option<u64>,
    /// Content-ID for inline images (cid: references).
    pub content_id: Option<String>,
    /// IMAP BODY section for fetching (e.g. "1.2").
    pub imap_section: String,
}
