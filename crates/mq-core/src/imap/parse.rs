//! Parse IMAP FETCH responses into domain types.

use crate::email::{Address, Email, MessageFlags};
use crate::imap::gmail_ext;

/// Parse an async-imap `Fetch` response into our `Email` domain type.
pub fn parse_fetch(fetch: &async_imap::types::Fetch) -> Option<Email> {
    let uid = fetch.uid?;

    let mut flags = MessageFlags::default();
    for flag in fetch.flags() {
        match flag {
            async_imap::types::Flag::Seen => flags.seen = true,
            async_imap::types::Flag::Flagged => flags.flagged = true,
            async_imap::types::Flag::Answered => flags.answered = true,
            async_imap::types::Flag::Deleted => flags.deleted = true,
            async_imap::types::Flag::Draft => flags.draft = true,
            _ => {}
        }
    }

    let (from, to, cc, bcc, subject, date, message_id, in_reply_to) =
        if let Some(env) = fetch.envelope() {
            let from = env
                .from
                .as_ref()
                .and_then(|addrs| addrs.first())
                .and_then(parse_imap_address);

            let to = env
                .to
                .as_ref()
                .map(|addrs| addrs.iter().filter_map(parse_imap_address).collect())
                .unwrap_or_default();

            let cc = env
                .cc
                .as_ref()
                .map(|addrs| addrs.iter().filter_map(parse_imap_address).collect())
                .unwrap_or_default();

            let bcc = env
                .bcc
                .as_ref()
                .map(|addrs| addrs.iter().filter_map(parse_imap_address).collect())
                .unwrap_or_default();

            let subject = env
                .subject
                .as_ref()
                .and_then(|s| decode_header_bytes(s));

            let date = env
                .date
                .as_ref()
                .and_then(|d| std::str::from_utf8(d).ok())
                .map(String::from);

            let message_id = env
                .message_id
                .as_ref()
                .and_then(|m| std::str::from_utf8(m).ok())
                .map(String::from);

            let in_reply_to = env
                .in_reply_to
                .as_ref()
                .and_then(|m| std::str::from_utf8(m).ok())
                .map(String::from);

            (from, to, cc, bcc, subject, date, message_id, in_reply_to)
        } else {
            (None, vec![], vec![], vec![], None, None, None, None)
        };

    // Extract list-unsubscribe headers from the fetched header fields
    let (list_unsubscribe, list_unsubscribe_post) =
        fetch
            .header()
            .map(parse_unsub_headers)
            .unwrap_or((None, None));

    // Extract Gmail extensions from the raw FETCH response.
    // async-imap doesn't parse these natively, so we use the Debug
    // representation to find them in the response data.
    let raw = format!("{:?}", fetch);
    let gmail_meta = gmail_ext::extract_gmail_metadata_from_raw(&raw);

    Some(Email {
        uid,
        gmail_msg_id: gmail_meta.gmail_msg_id,
        gmail_thread_id: gmail_meta.gmail_thread_id,
        message_id,
        in_reply_to,
        references: vec![], // Parsed from full headers when body is fetched
        from,
        to,
        cc,
        bcc,
        subject,
        date,
        snippet: None, // Generated when body is fetched
        flags,
        labels: gmail_meta.labels,
        has_attachments: false, // Determined from BODYSTRUCTURE (TODO)
        list_unsubscribe,
        list_unsubscribe_post,
    })
}

/// Parse flags from a FETCH response (for flag-only updates).
pub fn parse_flags(fetch: &async_imap::types::Fetch) -> Option<(u32, MessageFlags)> {
    let uid = fetch.uid?;
    let mut flags = MessageFlags::default();
    for flag in fetch.flags() {
        match flag {
            async_imap::types::Flag::Seen => flags.seen = true,
            async_imap::types::Flag::Flagged => flags.flagged = true,
            async_imap::types::Flag::Answered => flags.answered = true,
            async_imap::types::Flag::Deleted => flags.deleted = true,
            async_imap::types::Flag::Draft => flags.draft = true,
            _ => {}
        }
    }
    Some((uid, flags))
}

/// Convert an imap_proto Address to our domain Address.
fn parse_imap_address(addr: &async_imap::imap_proto::types::Address) -> Option<Address> {
    let mailbox = addr.mailbox.as_ref()?;
    let host = addr.host.as_ref()?;

    let mailbox_str = std::str::from_utf8(mailbox).ok()?;
    let host_str = std::str::from_utf8(host).ok()?;
    let email = format!("{mailbox_str}@{host_str}");

    let name = addr
        .name
        .as_ref()
        .and_then(|n| decode_header_bytes(n));

    Some(Address { name, email })
}

/// Decode potentially MIME-encoded header bytes into a String.
fn decode_header_bytes(bytes: &[u8]) -> Option<String> {
    // First try UTF-8
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some(s.to_string());
    }
    // Fall back to lossy conversion
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// Parse List-Unsubscribe and List-Unsubscribe-Post from raw header bytes.
fn parse_unsub_headers(header_bytes: &[u8]) -> (Option<String>, Option<String>) {
    let header_str = String::from_utf8_lossy(header_bytes);
    let mut list_unsub = None;
    let mut list_unsub_post = None;

    for line in header_str.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("list-unsubscribe-post:") {
            list_unsub_post = Some(line[22..].trim().to_string());
        } else if lower.starts_with("list-unsubscribe:") {
            list_unsub = Some(line[17..].trim().to_string());
        }
    }

    (list_unsub, list_unsub_post)
}
