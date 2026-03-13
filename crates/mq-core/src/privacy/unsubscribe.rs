//! Email unsubscribe support (RFC 8058 + RFC 2369).
//!
//! Parses `List-Unsubscribe` and `List-Unsubscribe-Post` headers.
//! Supports one-click HTTP POST (RFC 8058), mailto, and HTTPS fallback.

use crate::error::Result;

/// Parsed unsubscribe options from email headers.
#[derive(Debug, Clone)]
pub struct UnsubscribeInfo {
    /// The original List-Unsubscribe header value.
    pub raw_header: String,
    /// Parsed URLs (http/https) from the header.
    pub https_urls: Vec<String>,
    /// Parsed mailto: addresses from the header.
    pub mailto_addresses: Vec<String>,
    /// Whether RFC 8058 one-click POST is supported
    /// (List-Unsubscribe-Post: List-Unsubscribe=One-Click).
    pub supports_one_click: bool,
}

/// The recommended unsubscribe action to take.
#[derive(Debug, Clone)]
pub enum UnsubscribeAction {
    /// RFC 8058: HTTP POST to the URL with body `List-Unsubscribe=One-Click`.
    /// This is the preferred method — no user interaction needed beyond confirmation.
    OneClickPost { url: String },
    /// Send an email to the mailto address to unsubscribe.
    Mailto { address: String },
    /// Open an HTTPS URL in the browser for manual unsubscribe.
    OpenInBrowser { url: String },
}

impl UnsubscribeInfo {
    /// Parse List-Unsubscribe and List-Unsubscribe-Post headers.
    ///
    /// The `List-Unsubscribe` header contains a comma-separated list of
    /// angle-bracket-enclosed URLs, e.g.:
    /// `<https://example.com/unsub?id=abc>, <mailto:unsub@example.com?subject=unsubscribe>`
    pub fn parse(
        list_unsubscribe: &str,
        list_unsubscribe_post: Option<&str>,
    ) -> Self {
        let mut https_urls = Vec::new();
        let mut mailto_addresses = Vec::new();

        // Parse angle-bracket-enclosed URIs
        for part in list_unsubscribe.split(',') {
            let trimmed = part.trim();
            let uri = if trimmed.starts_with('<') && trimmed.ends_with('>') {
                &trimmed[1..trimmed.len() - 1]
            } else {
                trimmed
            };

            let uri_trimmed = uri.trim();
            if uri_trimmed.is_empty() {
                continue;
            }

            let lower = uri_trimmed.to_lowercase();
            if lower.starts_with("https://") || lower.starts_with("http://") {
                https_urls.push(uri_trimmed.to_string());
            } else if lower.starts_with("mailto:") {
                mailto_addresses.push(uri_trimmed.to_string());
            }
        }

        // RFC 8058: check if one-click POST is supported
        let supports_one_click = list_unsubscribe_post
            .map(|post| {
                post.to_lowercase()
                    .contains("list-unsubscribe=one-click")
            })
            .unwrap_or(false);

        UnsubscribeInfo {
            raw_header: list_unsubscribe.to_string(),
            https_urls,
            mailto_addresses,
            supports_one_click,
        }
    }

    /// Determine the best unsubscribe action to take.
    ///
    /// Priority:
    /// 1. RFC 8058 one-click POST (if supported + HTTPS URL available)
    /// 2. Mailto (send an email)
    /// 3. HTTPS URL (open in browser)
    pub fn recommended_action(&self) -> Option<UnsubscribeAction> {
        // 1. Prefer one-click POST (RFC 8058)
        if self.supports_one_click {
            if let Some(url) = self.https_urls.first() {
                return Some(UnsubscribeAction::OneClickPost {
                    url: url.clone(),
                });
            }
        }

        // 2. Mailto
        if let Some(addr) = self.mailto_addresses.first() {
            return Some(UnsubscribeAction::Mailto {
                address: addr.clone(),
            });
        }

        // 3. HTTPS fallback
        if let Some(url) = self.https_urls.first() {
            return Some(UnsubscribeAction::OpenInBrowser {
                url: url.clone(),
            });
        }

        None
    }

    /// Check if any unsubscribe option is available.
    pub fn has_options(&self) -> bool {
        !self.https_urls.is_empty() || !self.mailto_addresses.is_empty()
    }
}

/// Execute a one-click unsubscribe via HTTP POST (RFC 8058).
///
/// Sends a POST request to the given URL with body
/// `List-Unsubscribe=One-Click` and Content-Type `application/x-www-form-urlencoded`.
pub async fn one_click_unsubscribe(url: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("List-Unsubscribe=One-Click")
        .send()
        .await
        .map_err(|e| crate::error::MqError::Network(format!("Unsubscribe POST failed: {e}")))?;

    if response.status().is_success() || response.status().as_u16() == 302 {
        Ok(())
    } else {
        Err(crate::error::MqError::Network(format!(
            "Unsubscribe POST returned status {}",
            response.status()
        )))
    }
}

/// Parse a mailto: URI into (address, subject, body).
pub fn parse_mailto(mailto_uri: &str) -> Option<(String, Option<String>, Option<String>)> {
    let stripped = mailto_uri.strip_prefix("mailto:")?;

    let (address, query) = if let Some(pos) = stripped.find('?') {
        (&stripped[..pos], Some(&stripped[pos + 1..]))
    } else {
        (stripped, None)
    };

    if address.is_empty() {
        return None;
    }

    let mut subject = None;
    let mut body = None;

    if let Some(query) = query {
        for param in query.split('&') {
            if let Some((key, value)) = param.split_once('=') {
                match key.to_lowercase().as_str() {
                    "subject" => subject = Some(url_decode(value)),
                    "body" => body = Some(url_decode(value)),
                    _ => {}
                }
            }
        }
    }

    Some((address.to_string(), subject, body))
}

/// Simple percent-decode for mailto URI parameters.
fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rfc_8058_one_click() {
        let header = "<https://example.com/unsub?id=abc>, <mailto:unsub@example.com>";
        let post = "List-Unsubscribe=One-Click";
        let info = UnsubscribeInfo::parse(header, Some(post));

        assert!(info.supports_one_click);
        assert_eq!(info.https_urls.len(), 1);
        assert_eq!(info.mailto_addresses.len(), 1);

        let action = info.recommended_action().unwrap();
        assert!(matches!(action, UnsubscribeAction::OneClickPost { .. }));
    }

    #[test]
    fn test_parse_mailto_only() {
        let header = "<mailto:unsubscribe@lists.example.com?subject=unsubscribe>";
        let info = UnsubscribeInfo::parse(header, None);

        assert!(!info.supports_one_click);
        assert!(info.https_urls.is_empty());
        assert_eq!(info.mailto_addresses.len(), 1);

        let action = info.recommended_action().unwrap();
        assert!(matches!(action, UnsubscribeAction::Mailto { .. }));
    }

    #[test]
    fn test_parse_https_fallback() {
        let header = "<https://example.com/unsubscribe?token=xyz>";
        let info = UnsubscribeInfo::parse(header, None);

        assert!(!info.supports_one_click);
        assert_eq!(info.https_urls.len(), 1);

        let action = info.recommended_action().unwrap();
        assert!(matches!(action, UnsubscribeAction::OpenInBrowser { .. }));
    }

    #[test]
    fn test_parse_empty() {
        let info = UnsubscribeInfo::parse("", None);
        assert!(!info.has_options());
        assert!(info.recommended_action().is_none());
    }

    #[test]
    fn test_parse_mailto_uri() {
        let (addr, subj, body) =
            parse_mailto("mailto:unsub@example.com?subject=unsubscribe&body=please%20remove")
                .unwrap();
        assert_eq!(addr, "unsub@example.com");
        assert_eq!(subj.unwrap(), "unsubscribe");
        assert_eq!(body.unwrap(), "please remove");
    }

    #[test]
    fn test_parse_mailto_no_params() {
        let (addr, subj, body) = parse_mailto("mailto:unsub@example.com").unwrap();
        assert_eq!(addr, "unsub@example.com");
        assert!(subj.is_none());
        assert!(body.is_none());
    }

    #[test]
    fn test_multiple_urls() {
        let header = "<https://a.com/unsub>, <https://b.com/unsub>, <mailto:c@example.com>";
        let info = UnsubscribeInfo::parse(header, None);
        assert_eq!(info.https_urls.len(), 2);
        assert_eq!(info.mailto_addresses.len(), 1);
    }
}
