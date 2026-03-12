use serde::{Deserialize, Serialize};

/// Database row for an account.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct DbAccount {
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub created_at: String,
    pub last_sync: Option<String>,
}

/// Database row for a label (Gmail label / IMAP folder).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct DbLabel {
    pub id: i64,
    pub account_id: i64,
    pub name: String,
    pub imap_name: String,
    pub label_type: String,
    pub color: Option<String>,
    pub unread_count: i64,
    pub total_count: i64,
}

/// Database row for a message (header cache).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct DbMessage {
    pub id: i64,
    pub account_id: i64,
    pub uid: i64,
    pub mailbox: String,
    pub gmail_msg_id: Option<i64>,
    pub gmail_thread_id: Option<i64>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_json: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: String,
    pub recipient_to: String,
    pub recipient_cc: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub date: String,
    pub flags: String,
    pub has_attachments: bool,
    pub body_structure: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub modseq: Option<i64>,
    pub uid_validity: i64,
    pub cached_at: String,
}

/// Database row for a cached message body.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DbMessageBody {
    pub message_id: i64,
    pub raw_mime: Option<Vec<u8>>,
    pub html_body: Option<String>,
    pub text_body: Option<String>,
    pub fetched_at: String,
}

/// Database row for an attachment.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct DbAttachment {
    pub id: i64,
    pub message_id: i64,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size: Option<i64>,
    pub content_id: Option<String>,
    pub imap_section: String,
}

/// Database row for a sender in the image allowlist.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DbSenderAllowlist {
    pub id: i64,
    pub account_id: i64,
    pub sender_email: String,
    pub added_at: String,
}

/// Database row for an offline operation.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct DbOfflineOp {
    pub id: i64,
    pub account_id: i64,
    pub op_type: String,
    pub payload: String,
    pub status: String,
    pub retry_count: i64,
    pub created_at: String,
    pub last_attempt: Option<String>,
    pub error_msg: Option<String>,
}

/// Database row for sync state per mailbox.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DbSyncState {
    pub id: i64,
    pub account_id: i64,
    pub mailbox: String,
    pub uid_validity: i64,
    pub highest_modseq: i64,
    pub highest_uid: i64,
    pub last_sync: Option<String>,
}
