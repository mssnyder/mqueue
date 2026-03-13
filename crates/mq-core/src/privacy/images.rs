//! Remote image blocking and tracking pixel detection.
//!
//! - Blocks all remote images by default via CSS injection
//! - Detects and removes tracking pixels (1x1, known tracker domains, unique URL IDs)
//! - Per-sender "always load images" allowlist (managed in DB)

use url::Url;

/// Known tracking pixel / email analytics domains.
const TRACKER_DOMAINS: &[&str] = &[
    "open.convertkit.com",
    "pixel.mailchimp.com",
    "links.mlsend.com",
    "t.sendinblue.com",
    "t.sidekickopen.com",
    "track.hubspot.com",
    "tracking.tldcrm.com",
    "mailtrack.io",
    "bl-1.com",
    "list-manage.com",
    "mandrillapp.com",
    "sendgrid.net",
    "cmail19.com",
    "cmail20.com",
    "emltrk.com",
    "yesware.com",
    "bananatag.com",
    "getnotify.com",
    "returnpath.net",
    "litmus.com",
    "google-analytics.com",
    "facebook.com/tr",
    "bat.bing.com",
];

/// Result of sanitizing an HTML email body.
#[derive(Debug, Clone)]
pub struct SanitizedHtml {
    /// The sanitized HTML with remote images blocked.
    pub html: String,
    /// Number of remote images that were blocked.
    pub blocked_image_count: usize,
    /// Number of tracking pixels detected and removed.
    pub tracking_pixel_count: usize,
}

/// Sanitize HTML email body: block remote images and detect tracking pixels.
///
/// When `block_images` is true, remote `<img>` `src` attributes are replaced
/// with a placeholder and a `data-blocked-src` attribute preserves the original URL.
///
/// Tracking pixels are **always** removed regardless of settings — they are
/// never useful to the user and exist solely for surveillance.
pub fn sanitize_html(
    html: &str,
    block_images: bool,
    _detect_tracking_pixels: bool,
) -> SanitizedHtml {
    let mut result = String::with_capacity(html.len());
    let mut blocked_count = 0usize;
    let mut tracking_count = 0usize;

    let mut chars = html.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            // Collect the entire tag
            let mut tag = String::from('<');
            let mut depth = 1;
            for c in chars.by_ref() {
                tag.push(c);
                if c == '>' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                } else if c == '<' {
                    depth += 1;
                }
            }

            let tag_lower = tag.to_lowercase();

            // Handle <img> tags
            if tag_lower.starts_with("<img") {
                if let Some(src) = extract_attr(&tag, "src") {
                    if is_remote_url(&src) {
                        // Tracking pixels are always removed — unconditionally
                        let is_tracker = is_tracking_pixel(&tag, &src);

                        if is_tracker {
                            tracking_count += 1;
                            // Remove entirely — don't add to result
                            continue;
                        }

                        if block_images {
                            blocked_count += 1;
                            // Replace src with empty, preserve original in data attribute
                            let blocked_tag = replace_img_src(&tag, &src);
                            result.push_str(&blocked_tag);
                            continue;
                        }
                    }
                }
                result.push_str(&tag);
            }
            // Block remote loading via <link>, <style @import>, etc.
            else if tag_lower.starts_with("<link")
                && tag_lower.contains("stylesheet")
                && block_images
            {
                // Strip remote stylesheets that could load tracking resources
                if let Some(href) = extract_attr(&tag, "href") {
                    if is_remote_url(&href) {
                        continue; // Remove the tag
                    }
                }
                result.push_str(&tag);
            } else {
                result.push_str(&tag);
            }
        } else {
            result.push(ch);
        }
    }

    // If blocking images, inject CSP-like style at the start of <head> or <body>
    if block_images && blocked_count > 0 {
        let csp_style =
            "<style>img[data-blocked-src]{display:inline-block;min-width:16px;min-height:16px;\
             background:#f0f0f0;border:1px dashed #ccc;}</style>";

        // Try to inject after <head> or at the start
        if let Some(pos) = result.to_lowercase().find("<head>") {
            let insert_at = pos + "<head>".len();
            result.insert_str(insert_at, csp_style);
        } else if let Some(pos) = result.to_lowercase().find("<body") {
            // Find the end of the <body...> tag
            if let Some(end) = result[pos..].find('>') {
                let insert_at = pos + end + 1;
                result.insert_str(insert_at, csp_style);
            }
        }
    }

    SanitizedHtml {
        html: result,
        blocked_image_count: blocked_count,
        tracking_pixel_count: tracking_count,
    }
}

/// Check if a URL is a remote URL (http:// or https://).
fn is_remote_url(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://") || trimmed.starts_with("//")
}

/// Check if an image tag is likely a tracking pixel.
fn is_tracking_pixel(tag: &str, src: &str) -> bool {
    // 1. Check for 1x1 pixel dimensions in attributes
    let tag_lower = tag.to_lowercase();
    let is_tiny = (tag_lower.contains("width=\"1\"") || tag_lower.contains("width='1'"))
        && (tag_lower.contains("height=\"1\"") || tag_lower.contains("height='1'"));
    if is_tiny {
        return true;
    }

    // Also check for width:1px / height:1px in inline style
    if let Some(style) = extract_attr(tag, "style") {
        let style_lower = style.to_lowercase().replace(' ', "");
        if (style_lower.contains("width:1px") || style_lower.contains("width:0"))
            && (style_lower.contains("height:1px") || style_lower.contains("height:0"))
        {
            return true;
        }
    }

    // 2. Check against known tracker domains
    if let Ok(parsed) = Url::parse(src) {
        if let Some(host) = parsed.host_str() {
            for domain in TRACKER_DOMAINS {
                if host == *domain || host.ends_with(&format!(".{domain}")) {
                    return true;
                }
            }

            // 3. Heuristic: URL path contains very long unique identifiers (tracking tokens)
            let path = parsed.path();
            if path.len() > 80 {
                // Long paths with hex/base64 segments are likely tracking
                let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                for seg in &segments {
                    if seg.len() > 32 && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '=') {
                        return true;
                    }
                }
            }
        }
    }

    // 4. Common tracking pixel filenames
    let src_lower = src.to_lowercase();
    if src_lower.contains("/track") || src_lower.contains("/open") || src_lower.contains("/pixel")
        || src_lower.contains("/beacon") || src_lower.contains("/wf/open")
        || src_lower.ends_with(".gif") && is_tiny_by_name(&src_lower)
    {
        // Only flag as tracking if also has suspicious characteristics
        if src_lower.contains("?") || src_lower.contains("&") {
            return true;
        }
    }

    false
}

/// Check if a GIF URL name suggests a tracking pixel.
fn is_tiny_by_name(src: &str) -> bool {
    src.contains("spacer") || src.contains("blank") || src.contains("pixel") || src.contains("1x1")
}

/// Extract an attribute value from an HTML tag string.
fn extract_attr(tag: &str, attr_name: &str) -> Option<String> {
    let tag_lower = tag.to_lowercase();
    let search = format!("{attr_name}=");

    let start = tag_lower.find(&search)?;
    let after_eq = start + search.len();

    let bytes = tag.as_bytes();
    if after_eq >= bytes.len() {
        return None;
    }

    let quote = bytes[after_eq] as char;
    if quote == '"' || quote == '\'' {
        let value_start = after_eq + 1;
        let value_end = tag[value_start..].find(quote)?;
        Some(tag[value_start..value_start + value_end].to_string())
    } else {
        // Unquoted attribute — take until whitespace or >
        let value_start = after_eq;
        let value_end = tag[value_start..]
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(tag.len() - value_start);
        Some(tag[value_start..value_start + value_end].to_string())
    }
}

/// Replace an <img> tag's src with a blocked placeholder.
fn replace_img_src(tag: &str, original_src: &str) -> String {
    // Replace src="..." with src="" data-blocked-src="..."
    let replacement = format!(
        "src=\"\" data-blocked-src=\"{}\"",
        original_src.replace('"', "&quot;")
    );

    // Find and replace the src attribute
    if let Some(pos) = tag.to_lowercase().find("src=") {
        let after_eq = pos + 4;
        let bytes = tag.as_bytes();
        if after_eq < bytes.len() {
            let quote = bytes[after_eq] as char;
            if quote == '"' || quote == '\'' {
                let value_start = after_eq + 1;
                if let Some(value_end) = tag[value_start..].find(quote) {
                    let end = value_start + value_end + 1; // include closing quote
                    return format!("{}{}{}", &tag[..pos], replacement, &tag[end..]);
                }
            }
        }
    }

    // Fallback: return original
    tag.to_string()
}

/// Convert sanitized HTML to plain text summary (for display without WebKitGTK).
pub fn html_to_plain_text(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_space = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
            }
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !last_was_space {
                        result.push(' ');
                        last_was_space = true;
                    }
                } else {
                    result.push(ch);
                    last_was_space = false;
                }
            }
            _ => {}
        }
    }

    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blocks_remote_images() {
        let html = r#"<html><body><img src="https://example.com/image.jpg" alt="photo"></body></html>"#;
        let result = sanitize_html(html, true, false);
        assert_eq!(result.blocked_image_count, 1);
        assert!(result.html.contains("data-blocked-src=\"https://example.com/image.jpg\""));
        assert!(result.html.contains(r#"src="""#));
    }

    #[test]
    fn test_allows_data_uri_images() {
        let html = r#"<img src="data:image/png;base64,abc123">"#;
        let result = sanitize_html(html, true, false);
        assert_eq!(result.blocked_image_count, 0);
        assert!(result.html.contains("data:image/png;base64,abc123"));
    }

    #[test]
    fn test_detects_1x1_tracking_pixel() {
        let html = r#"<img src="https://tracker.com/pixel.gif" width="1" height="1">"#;
        let result = sanitize_html(html, false, true);
        assert_eq!(result.tracking_pixel_count, 1);
        assert!(!result.html.contains("tracker.com"));
    }

    #[test]
    fn test_detects_known_tracker_domain() {
        let html = r#"<img src="https://pixel.mailchimp.com/open/abc123">"#;
        let result = sanitize_html(html, false, true);
        assert_eq!(result.tracking_pixel_count, 1);
    }

    #[test]
    fn test_passthrough_when_disabled() {
        let html = r#"<img src="https://example.com/image.jpg">"#;
        let result = sanitize_html(html, false, false);
        assert_eq!(result.blocked_image_count, 0);
        assert_eq!(result.tracking_pixel_count, 0);
        assert!(result.html.contains("https://example.com/image.jpg"));
    }

    #[test]
    fn test_html_to_plain_text() {
        let html = "<p>Hello <b>world</b>!</p><p>Second paragraph.</p>";
        let text = html_to_plain_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("<p>"));
    }

    #[test]
    fn test_extract_attr() {
        let tag = r#"<img src="https://example.com/img.jpg" alt="test">"#;
        assert_eq!(
            extract_attr(tag, "src"),
            Some("https://example.com/img.jpg".to_string())
        );
        assert_eq!(extract_attr(tag, "alt"), Some("test".to_string()));
    }
}
