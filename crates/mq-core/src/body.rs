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
        let mut end = 200;
        // Try to break at a word boundary
        if let Some(pos) = trimmed[..200].rfind(' ') {
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
