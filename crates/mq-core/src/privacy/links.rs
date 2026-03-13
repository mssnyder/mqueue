//! Link tracking parameter stripping.
//!
//! Removes UTM parameters, fbclid, gclid, and other tracking query
//! parameters from URLs before opening them in the system browser.

use url::Url;

/// Tracking query parameters to strip from URLs.
const TRACKING_PARAMS: &[&str] = &[
    // Google Analytics / UTM
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "utm_source_platform",
    "utm_creative_format",
    "utm_marketing_tactic",
    // Facebook
    "fbclid",
    "fb_action_ids",
    "fb_action_types",
    "fb_ref",
    "fb_source",
    // Google Ads
    "gclid",
    "gclsrc",
    "dclid",
    "gbraid",
    "wbraid",
    // Microsoft / Bing
    "msclkid",
    // HubSpot
    "_hsenc",
    "_hsmi",
    "__hssc",
    "__hstc",
    "__hsfp",
    "hsCtaTracking",
    // Mailchimp
    "mc_eid",
    "mc_cid",
    // Drip
    "__s",
    // Marketo
    "mkt_tok",
    // Vero
    "vero_id",
    "vero_conv",
    // Campaign Monitor
    "cm_mmc",
    // Adobe
    "sc_cid",
    "ef_id",
    "s_kwcid",
    // Generic
    "trk",
    "trkCampaign",
    "trkInfo",
    "ref_",
];

/// Strip tracking parameters from a URL.
///
/// Returns the cleaned URL string. If the URL cannot be parsed or has
/// no tracking parameters, it is returned unchanged.
pub fn strip_tracking_params(url_str: &str) -> String {
    let Ok(mut parsed) = Url::parse(url_str) else {
        return url_str.to_string();
    };

    let original_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    if original_pairs.is_empty() {
        return url_str.to_string();
    }

    let filtered: Vec<(&str, &str)> = original_pairs
        .iter()
        .filter(|(key, _)| !is_tracking_param(key))
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    // If nothing was filtered, return original to avoid unnecessary re-encoding
    if filtered.len() == original_pairs.len() {
        return url_str.to_string();
    }

    if filtered.is_empty() {
        parsed.set_query(None);
    } else {
        let new_query: String = filtered
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    k.to_string()
                } else {
                    format!("{k}={v}")
                }
            })
            .collect::<Vec<_>>()
            .join("&");
        parsed.set_query(Some(&new_query));
    }

    parsed.to_string()
}

/// Check if a query parameter name is a known tracking parameter.
fn is_tracking_param(name: &str) -> bool {
    let lower = name.to_lowercase();
    TRACKING_PARAMS
        .iter()
        .any(|&param| lower == param.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strips_utm_params() {
        let url = "https://example.com/page?utm_source=newsletter&utm_medium=email&id=123";
        let cleaned = strip_tracking_params(url);
        assert!(cleaned.contains("id=123"));
        assert!(!cleaned.contains("utm_source"));
        assert!(!cleaned.contains("utm_medium"));
    }

    #[test]
    fn test_strips_fbclid() {
        let url = "https://example.com/page?article=42&fbclid=abc123def456";
        let cleaned = strip_tracking_params(url);
        assert!(cleaned.contains("article=42"));
        assert!(!cleaned.contains("fbclid"));
    }

    #[test]
    fn test_strips_all_tracking_leaves_no_query() {
        let url = "https://example.com/page?utm_source=test&utm_medium=email";
        let cleaned = strip_tracking_params(url);
        assert_eq!(cleaned, "https://example.com/page");
    }

    #[test]
    fn test_preserves_non_tracking_params() {
        let url = "https://example.com/search?q=hello&page=2";
        let cleaned = strip_tracking_params(url);
        assert_eq!(cleaned, url);
    }

    #[test]
    fn test_no_query_string() {
        let url = "https://example.com/page";
        let cleaned = strip_tracking_params(url);
        assert_eq!(cleaned, url);
    }

    #[test]
    fn test_invalid_url_passthrough() {
        let url = "not a valid url";
        let cleaned = strip_tracking_params(url);
        assert_eq!(cleaned, url);
    }

    #[test]
    fn test_strips_gclid() {
        let url = "https://shop.example.com/product?id=99&gclid=xyz789&color=blue";
        let cleaned = strip_tracking_params(url);
        assert!(cleaned.contains("id=99"));
        assert!(cleaned.contains("color=blue"));
        assert!(!cleaned.contains("gclid"));
    }

    #[test]
    fn test_strips_hubspot_params() {
        let url = "https://example.com/page?_hsenc=abc&_hsmi=def&real=param";
        let cleaned = strip_tracking_params(url);
        assert!(cleaned.contains("real=param"));
        assert!(!cleaned.contains("_hsenc"));
        assert!(!cleaned.contains("_hsmi"));
    }
}
