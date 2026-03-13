//! Parse IMAP FETCH responses into domain types.

use crate::email::{Address, Email, MessageFlags};

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

    // Extract Gmail extensions using async-imap's native accessors.
    // gmail_msg_id() and gmail_labels() are provided directly.
    // gmail_thread_id (X-GM-THRID) is parsed by imap-proto but async-imap
    // doesn't expose an accessor, so we extract it from the parsed attributes.
    let gmail_msg_id = fetch.gmail_msg_id().copied();
    let gmail_thread_id = extract_gmail_thread_id(fetch);
    let labels = fetch
        .gmail_labels()
        .map(|ls| ls.iter().map(|l| l.to_string()).collect())
        .unwrap_or_default();

    Some(Email {
        uid,
        gmail_msg_id,
        gmail_thread_id,
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
        labels,
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
///
/// Handles RFC 2047 encoded-words (e.g. `=?UTF-8?Q?Hello?=`) which appear
/// in IMAP ENVELOPE fields for non-ASCII subjects and sender names.
fn decode_header_bytes(bytes: &[u8]) -> Option<String> {
    // First try UTF-8
    let raw = if let Ok(s) = std::str::from_utf8(bytes) {
        s.to_string()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };

    // Check for RFC 2047 encoded-words (=?charset?encoding?text?=)
    if raw.contains("=?") {
        if let Some(decoded) = decode_rfc2047(&raw) {
            return Some(decoded);
        }
    }

    Some(raw)
}

/// Decode RFC 2047 encoded-word strings.
///
/// Supports Q-encoding and B-encoding (base64) with any charset that
/// `encoding_rs` handles (UTF-8, ISO-8859-*, etc.).
fn decode_rfc2047(input: &str) -> Option<String> {
    use encoding_rs::Encoding;

    let mut result = String::with_capacity(input.len());
    let mut remaining = input;

    while !remaining.is_empty() {
        if let Some(start) = remaining.find("=?") {
            // Add text before the encoded-word
            result.push_str(&remaining[..start]);
            let after_start = &remaining[start + 2..];

            // Find charset?encoding?text?=
            let parts: Vec<&str> = after_start.splitn(4, '?').collect();
            if parts.len() >= 3 {
                let charset = parts[0];
                let encoding = parts[1];
                // The encoded text ends at "?="
                if let Some(end_idx) = parts[2..].join("?").find("?=") {
                    let encoded_text = &parts[2..].join("?")[..end_idx];
                    let decoded_bytes = match encoding.to_uppercase().as_str() {
                        "Q" => decode_q_encoding(encoded_text),
                        "B" => {
                            use base64::Engine;
                            base64::engine::general_purpose::STANDARD
                                .decode(encoded_text)
                                .ok()
                        }
                        _ => None,
                    };

                    if let Some(bytes) = decoded_bytes {
                        // Convert from charset to UTF-8
                        if charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("utf8") {
                            result.push_str(&String::from_utf8_lossy(&bytes));
                        } else if let Some(enc) = Encoding::for_label(charset.as_bytes()) {
                            let (decoded, _, _) = enc.decode(&bytes);
                            result.push_str(&decoded);
                        } else {
                            result.push_str(&String::from_utf8_lossy(&bytes));
                        }

                        // Skip past the encoded-word (=?charset?encoding?text?=)
                        let consumed = start + 2 + charset.len() + 1 + encoding.len() + 1 + end_idx + 2;
                        remaining = &remaining[consumed..];

                        // Skip whitespace between consecutive encoded-words
                        remaining = remaining.trim_start();
                        continue;
                    }
                }
            }

            // Failed to parse — keep the literal text and advance past "=?"
            result.push_str("=?");
            remaining = after_start;
        } else {
            result.push_str(remaining);
            break;
        }
    }

    Some(result)
}

/// Decode Q-encoding (RFC 2047): underscores → spaces, =XX → byte
fn decode_q_encoding(input: &str) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'_' => {
                result.push(b' ');
                i += 1;
            }
            b'=' if i + 2 < bytes.len() => {
                let hex = &input[i + 1..i + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    result.push(byte);
                    i += 3;
                } else {
                    result.push(b'=');
                    i += 1;
                }
            }
            b => {
                result.push(b);
                i += 1;
            }
        }
    }
    Some(result)
}

/// Extract Gmail thread ID (X-GM-THRID) from a Fetch response.
///
/// async-imap 0.11.2 doesn't expose a `gmail_thr_id()` accessor even though
/// imap-proto parses X-GM-THRID into `AttributeValue::GmailThrId(u64)`.
/// We extract it from the Debug representation where it appears as `GmailThrId(12345)`.
fn extract_gmail_thread_id(fetch: &async_imap::types::Fetch) -> Option<u64> {
    let debug = format!("{:?}", fetch);
    let marker = "GmailThrId(";
    let idx = debug.find(marker)?;
    let rest = &debug[idx + marker.len()..];
    let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
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
