//! Gmail-specific IMAP extensions.
//!
//! - X-GM-RAW: Server-side search using Gmail's search syntax
//! - X-GM-LABELS: Read and mutate Gmail labels
//! - X-GM-MSGID / X-GM-THRID: Gmail message and thread identity
//!
//! These extensions are not part of standard IMAP but are supported by
//! Gmail's IMAP server. async-imap doesn't parse them natively, so we
//! work with raw command/response handling where needed.

use futures::TryStreamExt;
use tracing::debug;

use crate::error::Result;
use crate::imap::client::ImapSession;

/// Gmail metadata for a single message, extracted from FETCH responses.
#[derive(Debug, Clone, Default)]
pub struct GmailMetadata {
    pub uid: u32,
    /// Gmail's globally unique message ID (X-GM-MSGID).
    pub gmail_msg_id: Option<u64>,
    /// Gmail's thread ID (X-GM-THRID).
    pub gmail_thread_id: Option<u64>,
    /// Gmail labels (X-GM-LABELS), e.g. ["\\Inbox", "Work", "Projects/Alpha"].
    pub labels: Vec<String>,
}

/// Search Gmail using X-GM-RAW (Gmail's full search syntax over IMAP).
///
/// This sends a UID SEARCH command with X-GM-RAW, which allows using Gmail's
/// search operators (from:, to:, subject:, has:attachment, label:, etc.).
///
/// Returns a list of matching UIDs.
pub async fn search_gmail(session: &mut ImapSession, query: &str) -> Result<Vec<u32>> {
    // X-GM-RAW wraps the query in double quotes
    let search_cmd = format!("X-GM-RAW \"{}\"", query.replace('"', "\\\""));
    debug!(%search_cmd, "Gmail server-side search");
    let uids = session.search(&search_cmd).await?;
    debug!(count = uids.len(), "Gmail search returned");
    Ok(uids)
}

/// Fetch Gmail-specific metadata (X-GM-MSGID, X-GM-THRID, X-GM-LABELS) for
/// messages in the given UID range.
pub async fn fetch_gmail_metadata(
    session: &mut ImapSession,
    uid_range: &str,
) -> Result<Vec<GmailMetadata>> {
    let fetches: Vec<_> = session
        .inner_mut()
        .uid_fetch(uid_range, "(UID X-GM-MSGID X-GM-THRID X-GM-LABELS)")
        .await?
        .try_collect()
        .await?;

    let mut results = Vec::with_capacity(fetches.len());
    for fetch in &fetches {
        let uid = match fetch.uid {
            Some(uid) => uid,
            None => continue,
        };

        // async-imap's Fetch type doesn't expose Gmail extensions directly,
        // so we parse the raw response bytes for the extension data.
        let raw = format!("{:?}", fetch);
        let gmail_msg_id = parse_gmail_id(&raw, "X-GM-MSGID");
        let gmail_thread_id = parse_gmail_id(&raw, "X-GM-THRID");
        let labels = parse_gmail_labels(&raw);

        results.push(GmailMetadata {
            uid,
            gmail_msg_id,
            gmail_thread_id,
            labels,
        });
    }

    debug!(count = results.len(), uid_range, "Fetched Gmail metadata");
    Ok(results)
}

/// Add or remove Gmail labels on a message using X-GM-LABELS STORE.
pub async fn store_labels(
    session: &mut ImapSession,
    uid: u32,
    labels: &[&str],
    add: bool,
) -> Result<()> {
    let label_list: Vec<String> = labels
        .iter()
        .map(|l| {
            // Labels with spaces or special chars need quoting
            if l.contains(' ') || l.contains('/') || l.contains('"') {
                format!("\"{}\"", l.replace('"', "\\\""))
            } else {
                l.to_string()
            }
        })
        .collect();
    let label_str = label_list.join(" ");

    let cmd = if add {
        format!("+X-GM-LABELS ({label_str})")
    } else {
        format!("-X-GM-LABELS ({label_str})")
    };

    let _responses: Vec<_> = session
        .inner_mut()
        .uid_store(uid.to_string(), &cmd)
        .await?
        .try_collect()
        .await?;

    debug!(uid, %cmd, "Stored Gmail labels");
    Ok(())
}

/// Extract Gmail metadata from a raw FETCH response debug string.
///
/// Used by `parse.rs` to pull Gmail extension data out of FETCH responses
/// without needing an IMAP session. Returns a partial `GmailMetadata` (uid=0).
pub fn extract_gmail_metadata_from_raw(raw: &str) -> GmailMetadata {
    GmailMetadata {
        uid: 0,
        gmail_msg_id: parse_gmail_id(raw, "X-GM-MSGID"),
        gmail_thread_id: parse_gmail_id(raw, "X-GM-THRID"),
        labels: parse_gmail_labels(raw),
    }
}

/// Parse a numeric Gmail extension attribute (X-GM-MSGID or X-GM-THRID)
/// from a raw FETCH response debug string.
fn parse_gmail_id(raw: &str, attr: &str) -> Option<u64> {
    let marker = format!("{attr} ");
    let idx = raw.find(&marker)?;
    let rest = &raw[idx + marker.len()..];
    let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

/// Parse X-GM-LABELS from a raw FETCH response debug string.
///
/// Gmail labels are returned as a parenthesized list, e.g.:
///   X-GM-LABELS (\Inbox "Work" "Projects/Alpha")
fn parse_gmail_labels(raw: &str) -> Vec<String> {
    let marker = "X-GM-LABELS (";
    let idx = match raw.find(marker) {
        Some(i) => i,
        None => return vec![],
    };

    let rest = &raw[idx + marker.len()..];
    let end = match rest.find(')') {
        Some(i) => i,
        None => return vec![],
    };

    let label_str = &rest[..end];
    parse_label_list(label_str)
}

/// Parse a space-separated label list, handling quoted strings.
///
/// Input like: `\Inbox "Work" "Projects/Alpha" Unread`
/// Returns: `["\\Inbox", "Work", "Projects/Alpha", "Unread"]`
fn parse_label_list(input: &str) -> Vec<String> {
    let mut labels = Vec::new();
    let mut chars = input.chars().peekable();

    while chars.peek().is_some() {
        // Skip whitespace
        while chars.peek() == Some(&' ') {
            chars.next();
        }

        if chars.peek().is_none() {
            break;
        }

        if chars.peek() == Some(&'"') {
            // Quoted label
            chars.next(); // consume opening quote
            let mut label = String::new();
            loop {
                match chars.next() {
                    Some('\\') => {
                        if let Some(c) = chars.next() {
                            label.push(c);
                        }
                    }
                    Some('"') => break,
                    Some(c) => label.push(c),
                    None => break,
                }
            }
            if !label.is_empty() {
                labels.push(label);
            }
        } else {
            // Unquoted label (read until space or end)
            let mut label = String::new();
            while let Some(&c) = chars.peek() {
                if c == ' ' {
                    break;
                }
                label.push(c);
                chars.next();
            }
            if !label.is_empty() {
                labels.push(label);
            }
        }
    }

    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gmail_id() {
        let raw = "Fetch { uid: Some(42), X-GM-MSGID 1234567890 X-GM-THRID 9876543210 }";
        assert_eq!(parse_gmail_id(raw, "X-GM-MSGID"), Some(1234567890));
        assert_eq!(parse_gmail_id(raw, "X-GM-THRID"), Some(9876543210));
    }

    #[test]
    fn test_parse_gmail_id_missing() {
        let raw = "Fetch { uid: Some(42) }";
        assert_eq!(parse_gmail_id(raw, "X-GM-MSGID"), None);
    }

    #[test]
    fn test_parse_gmail_labels_simple() {
        let raw = r#"X-GM-LABELS (\Inbox Work)"#;
        let labels = parse_gmail_labels(raw);
        assert_eq!(labels, vec!["\\Inbox", "Work"]);
    }

    #[test]
    fn test_parse_gmail_labels_quoted() {
        let raw = r#"X-GM-LABELS (\Inbox "Projects/Alpha" "My Label")"#;
        let labels = parse_gmail_labels(raw);
        assert_eq!(labels, vec!["\\Inbox", "Projects/Alpha", "My Label"]);
    }

    #[test]
    fn test_parse_gmail_labels_empty() {
        let raw = "X-GM-LABELS ()";
        let labels = parse_gmail_labels(raw);
        assert!(labels.is_empty());
    }

    #[test]
    fn test_parse_gmail_labels_missing() {
        let raw = "Fetch { uid: Some(42) }";
        let labels = parse_gmail_labels(raw);
        assert!(labels.is_empty());
    }

    #[test]
    fn test_parse_label_list_mixed() {
        let labels = parse_label_list(r#"\Inbox "Work Items" Starred "A/B""#);
        assert_eq!(labels, vec!["\\Inbox", "Work Items", "Starred", "A/B"]);
    }

    #[test]
    fn test_parse_label_list_escaped_quote() {
        let labels = parse_label_list(r#""Label \"with\" quotes""#);
        assert_eq!(labels, vec!["Label \"with\" quotes"]);
    }
}
