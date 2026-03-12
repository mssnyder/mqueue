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
