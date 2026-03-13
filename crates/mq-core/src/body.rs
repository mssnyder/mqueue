//! Parse raw MIME email bodies using mail-parser.
//!
//! Extracts HTML body, text body, snippet, and attachment metadata
//! from raw RFC 5322 messages.

use mail_parser::MimeHeaders;

use crate::email::Attachment;

/// Parsed email body content.
#[derive(Debug, Clone)]
pub struct ParsedBody {
    /// The HTML body part, if present.
    pub html: Option<String>,
    /// The plain-text body part, if present.
    pub text: Option<String>,
    /// A short snippet of the text body (first ~200 chars).
    pub snippet: Option<String>,
    /// Attachment metadata extracted from the MIME structure.
    pub attachments: Vec<Attachment>,
}

/// Parse raw MIME bytes into body parts and attachment metadata.
pub fn parse_mime(raw: &[u8]) -> ParsedBody {
    let message = match mail_parser::MessageParser::default().parse(raw) {
        Some(msg) => msg,
        None => {
            return ParsedBody {
                html: None,
                text: None,
                snippet: None,
                attachments: vec![],
            };
        }
    };

    let html = message.body_html(0).map(|s| s.into_owned());
    let text = message.body_text(0).map(|s| s.into_owned());

    let snippet = text.as_deref().map(make_snippet);

    let mut attachments = Vec::new();
    for (idx, part) in message.parts.iter().enumerate() {
        // Skip the root multipart container and inline body parts
        if idx == 0 && message.parts.len() > 1 {
            continue;
        }

        // Check if this part is an attachment (not an inline body part)
        let disposition = part.content_disposition();
        let is_attachment = disposition
            .map(|d| d.ctype() == "attachment")
            .unwrap_or(false)
            || (!part.is_text() && !part.is_text_html() && disposition.is_some());

        if !is_attachment {
            continue;
        }

        let filename = part
            .attachment_name()
            .map(|s: &str| s.to_string());

        let mime_type = part
            .content_type()
            .map(|ct: &mail_parser::ContentType| {
                let main = ct.ctype();
                match ct.subtype() {
                    Some(sub) => format!("{main}/{sub}"),
                    None => main.to_string(),
                }
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let size = Some(part.contents().len() as u64);

        let content_id = part.content_id().map(|s: &str| s.to_string());

        attachments.push(Attachment {
            filename,
            mime_type,
            size,
            content_id,
            imap_section: idx.to_string(),
        });
    }

    ParsedBody {
        html,
        text,
        snippet,
        attachments,
    }
}

/// Resolve `cid:` image references in HTML by inlining them as `data:` URIs.
///
/// Parses the raw MIME to find inline parts with Content-ID headers, then
/// replaces any `src="cid:xxx"` in the HTML with `data:mime/type;base64,...`.
/// CID images are embedded content (not remote), so this is always safe.
pub fn resolve_cid_images(html: &str, raw_mime: &[u8]) -> String {
    use base64::Engine;
    use mail_parser::MimeHeaders;

    let message = match mail_parser::MessageParser::default().parse(raw_mime) {
        Some(msg) => msg,
        None => return html.to_string(),
    };

    // Collect all parts that have a Content-ID
    let mut cid_parts: Vec<(String, String, Vec<u8>)> = Vec::new();
    for part in &message.parts {
        if let Some(cid) = part.content_id() {
            let cid_clean = cid.trim_matches('<').trim_matches('>').to_string();
            let mime_type = part
                .content_type()
                .map(|ct| {
                    let main = ct.ctype();
                    match ct.subtype() {
                        Some(sub) => format!("{main}/{sub}"),
                        None => main.to_string(),
                    }
                })
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let data = part.contents().to_vec();
            if !data.is_empty() {
                cid_parts.push((cid_clean, mime_type, data));
            }
        }
    }

    if cid_parts.is_empty() {
        return html.to_string();
    }

    // Replace all cid: references in img src attributes
    let mut result = html.to_string();
    for (cid, mime_type, data) in &cid_parts {
        let b64 = base64::engine::general_purpose::STANDARD.encode(data);
        let data_uri = format!("data:{mime_type};base64,{b64}");

        // Replace src="cid:xxx" (with or without angle brackets)
        let patterns = [
            format!("src=\"cid:{cid}\""),
            format!("src='cid:{cid}'"),
            format!("src=\"cid:&lt;{cid}&gt;\""),
        ];
        for pat in &patterns {
            if result.contains(pat.as_str()) {
                result = result.replace(pat.as_str(), &format!("src=\"{data_uri}\""));
            }
        }
    }

    result
}

/// Extract the raw content of an attachment from a raw MIME message.
///
/// `section_index` is the part index stored in `Attachment::imap_section`.
pub fn extract_attachment_content(raw: &[u8], section_index: usize) -> Option<Vec<u8>> {
    let message = mail_parser::MessageParser::default().parse(raw)?;
    let part = message.parts.get(section_index)?;
    let contents = part.contents();
    if contents.is_empty() {
        None
    } else {
        Some(contents.to_vec())
    }
}

/// Generate a short snippet from body text (first ~200 chars, single line).
fn make_snippet(text: &str) -> String {
    let trimmed: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = trimmed.as_str();
    if trimmed.len() <= 200 {
        trimmed.to_string()
    } else {
        // Find a safe char boundary at or before byte 200
        let mut safe = 200;
        while safe > 0 && !trimmed.is_char_boundary(safe) {
            safe -= 1;
        }
        let mut end = safe;
        // Try to break at a word boundary
        if let Some(pos) = trimmed[..safe].rfind(' ') {
            end = pos;
        }
        format!("{}…", &trimmed[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_snippet_short() {
        assert_eq!(make_snippet("Hello world"), "Hello world");
    }

    #[test]
    fn test_make_snippet_long() {
        let text = "a ".repeat(200);
        let snippet = make_snippet(&text);
        assert!(snippet.len() <= 210);
        assert!(snippet.ends_with('…'));
    }

    #[test]
    fn test_make_snippet_normalizes_whitespace() {
        assert_eq!(make_snippet("Hello\n  world\ttab"), "Hello world tab");
    }

    #[test]
    fn test_parse_simple_text() {
        let raw = b"From: test@example.com\r\nTo: user@example.com\r\nSubject: Test\r\n\r\nHello, world!\r\n";
        let parsed = parse_mime(raw);
        assert!(parsed.text.is_some());
        assert!(parsed.text.unwrap().contains("Hello, world!"));
    }
}
