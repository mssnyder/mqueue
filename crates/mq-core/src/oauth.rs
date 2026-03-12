use std::net::TcpListener;

use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use tracing::{debug, info};

use crate::error::{MqError, Result};

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GMAIL_SCOPE: &str = "https://mail.google.com/";

/// OAuth2 client with auth and token endpoints configured.
pub type GoogleClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

/// Tokens obtained from the OAuth2 flow.
#[derive(Debug, Clone)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Seconds until the access token expires.
    pub expires_in: Option<u64>,
}

/// Build the OAuth2 client for Google.
pub fn build_client(
    client_id: &str,
    client_secret: &str,
    redirect_port: u16,
) -> Result<GoogleClient> {
    let redirect_url = format!("http://127.0.0.1:{redirect_port}/callback");

    let client = BasicClient::new(ClientId::new(client_id.to_string()))
        .set_client_secret(ClientSecret::new(client_secret.to_string()))
        .set_auth_uri(
            AuthUrl::new(GOOGLE_AUTH_URL.to_string())
                .map_err(|e| MqError::OAuth(format!("Invalid auth URL: {e}")))?,
        )
        .set_token_uri(
            TokenUrl::new(GOOGLE_TOKEN_URL.to_string())
                .map_err(|e| MqError::OAuth(format!("Invalid token URL: {e}")))?,
        )
        .set_redirect_uri(
            RedirectUrl::new(redirect_url)
                .map_err(|e| MqError::OAuth(format!("Invalid redirect URL: {e}")))?,
        );

    Ok(client)
}

/// Find a free port on localhost for the OAuth redirect.
pub fn find_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| MqError::OAuth(format!("Failed to bind to localhost: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| MqError::OAuth(format!("Failed to get local address: {e}")))?
        .port();
    Ok(port)
}

/// Generate the authorization URL that the user should visit.
///
/// Returns `(url, csrf_token, pkce_verifier)`.
pub fn authorization_url(
    client: &GoogleClient,
) -> (String, CsrfToken, oauth2::PkceCodeVerifier) {
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (auth_url, csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new(GMAIL_SCOPE.to_string()))
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .set_pkce_challenge(pkce_challenge)
        .url();

    (auth_url.to_string(), csrf_token, pkce_verifier)
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(
    client: &GoogleClient,
    code: String,
    pkce_verifier: oauth2::PkceCodeVerifier,
) -> Result<OAuthTokens> {
    let http_client = reqwest::Client::new();
    let token_result = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(&http_client)
        .await
        .map_err(|e| MqError::OAuth(format!("Token exchange failed: {e}")))?;

    let access_token = token_result.access_token().secret().clone();
    let refresh_token = token_result.refresh_token().map(|t| t.secret().clone());
    let expires_in = token_result.expires_in().map(|d| d.as_secs());

    info!("OAuth token exchange successful");
    debug!(
        expires_in = ?expires_in,
        has_refresh = refresh_token.is_some(),
        "Token details"
    );

    Ok(OAuthTokens {
        access_token,
        refresh_token,
        expires_in,
    })
}

/// Refresh an access token using a refresh token.
pub async fn refresh_access_token(
    client: &GoogleClient,
    refresh_token: &str,
) -> Result<OAuthTokens> {
    let http_client = reqwest::Client::new();
    let token_result = client
        .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token.to_string()))
        .request_async(&http_client)
        .await
        .map_err(|e| MqError::OAuth(format!("Token refresh failed: {e}")))?;

    let access_token = token_result.access_token().secret().clone();
    let new_refresh = token_result.refresh_token().map(|t| t.secret().clone());
    let expires_in = token_result.expires_in().map(|d| d.as_secs());

    info!("OAuth token refresh successful");

    Ok(OAuthTokens {
        access_token,
        refresh_token: new_refresh,
        expires_in,
    })
}

/// Build the XOAUTH2 SASL string for IMAP/SMTP authentication.
///
/// Format: `user=<email>\x01auth=Bearer <token>\x01\x01`
pub fn xoauth2_string(email: &str, access_token: &str) -> String {
    format!("user={email}\x01auth=Bearer {access_token}\x01\x01")
}
