//! Token storage via GNOME Keyring (oo7 / Secret Service).
//!
//! Stores OAuth2 access and refresh tokens securely, keyed by email address.

use std::collections::HashMap;

use tracing::{debug, info, warn};

use crate::error::{MqError, Result};

const SERVICE_NAME: &str = "mq-mail";

/// Stored OAuth tokens for an account.
#[derive(Debug, Clone)]
pub struct StoredTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

fn make_attrs<'a>(email: &'a str, token_type: &'a str) -> HashMap<&'a str, &'a str> {
    let mut m = HashMap::new();
    m.insert("service", SERVICE_NAME);
    m.insert("email", email);
    m.insert("type", token_type);
    m
}

/// Store OAuth tokens in the keyring for the given email.
pub async fn store_tokens(
    email: &str,
    access_token: &str,
    refresh_token: Option<&str>,
) -> Result<()> {
    let keyring = oo7::Keyring::new().await.map_err(|e| {
        MqError::Other(anyhow::anyhow!("Failed to open keyring: {e}"))
    })?;

    // Store access token
    keyring
        .create_item(
            &format!("m'Queue access token for {email}"),
            &make_attrs(email, "access_token"),
            access_token,
            true, // replace existing
        )
        .await
        .map_err(|e| MqError::Other(anyhow::anyhow!("Failed to store access token: {e}")))?;

    // Store refresh token if present
    if let Some(refresh) = refresh_token {
        keyring
            .create_item(
                &format!("m'Queue refresh token for {email}"),
                &make_attrs(email, "refresh_token"),
                refresh,
                true,
            )
            .await
            .map_err(|e| {
                MqError::Other(anyhow::anyhow!("Failed to store refresh token: {e}"))
            })?;
    }

    info!(email, "Stored tokens in keyring");
    Ok(())
}

/// Retrieve stored tokens from the keyring for the given email.
pub async fn get_tokens(email: &str) -> Result<Option<StoredTokens>> {
    let keyring = oo7::Keyring::new().await.map_err(|e| {
        MqError::Other(anyhow::anyhow!("Failed to open keyring: {e}"))
    })?;

    // Retrieve access token
    let access_items = keyring
        .search_items(&make_attrs(email, "access_token"))
        .await
        .map_err(|e| MqError::Other(anyhow::anyhow!("Failed to search keyring: {e}")))?;

    let access_token = match access_items.first() {
        Some(item) => {
            let secret = item.secret().await.map_err(|e| {
                MqError::Other(anyhow::anyhow!("Failed to read access token secret: {e}"))
            })?;
            String::from_utf8(secret.to_vec())
                .map_err(|e| MqError::Other(anyhow::anyhow!("Invalid access token: {e}")))?
        }
        None => {
            debug!(email, "No access token found in keyring");
            return Ok(None);
        }
    };

    // Retrieve refresh token (optional)
    let refresh_items = keyring
        .search_items(&make_attrs(email, "refresh_token"))
        .await
        .unwrap_or_default();

    let refresh_token = if let Some(item) = refresh_items.first() {
        match item.secret().await {
            Ok(secret) => String::from_utf8(secret.to_vec()).ok(),
            Err(e) => {
                warn!(email, error = %e, "Failed to read refresh token");
                None
            }
        }
    } else {
        None
    };

    debug!(email, has_refresh = refresh_token.is_some(), "Retrieved tokens from keyring");
    Ok(Some(StoredTokens {
        access_token,
        refresh_token,
    }))
}

/// Delete stored tokens for an email (used when removing an account).
pub async fn delete_tokens(email: &str) -> Result<()> {
    let keyring = oo7::Keyring::new().await.map_err(|e| {
        MqError::Other(anyhow::anyhow!("Failed to open keyring: {e}"))
    })?;

    for token_type in &["access_token", "refresh_token"] {
        let _ = keyring.delete(&make_attrs(email, token_type)).await;
    }

    info!(email, "Deleted tokens from keyring");
    Ok(())
}

/// Refresh the access token using the stored refresh token and update keyring.
pub async fn refresh_and_store(email: &str) -> Result<String> {
    let tokens = get_tokens(email)
        .await?
        .ok_or_else(|| MqError::OAuth(format!("No tokens stored for {email}")))?;

    let refresh_token = tokens
        .refresh_token
        .ok_or_else(|| MqError::OAuth(format!("No refresh token stored for {email}")))?;

    let config = crate::config::AppConfig::load().unwrap_or_default();
    let client_id = config
        .resolve_client_id()?
        .ok_or_else(|| MqError::OAuth("OAuth client_id not configured".into()))?;
    let client_secret = config
        .resolve_client_secret()?
        .ok_or_else(|| MqError::OAuth("OAuth client_secret not configured".into()))?;

    // Use port 0 since we don't need a redirect for refresh
    let client = crate::oauth::build_client(&client_id, &client_secret, 0)?;
    let new_tokens = crate::oauth::refresh_access_token(&client, &refresh_token).await?;

    // Store updated tokens
    store_tokens(
        email,
        &new_tokens.access_token,
        new_tokens.refresh_token.as_deref().or(Some(&refresh_token)),
    )
    .await?;

    info!(email, "Refreshed and stored new access token");
    Ok(new_tokens.access_token)
}
