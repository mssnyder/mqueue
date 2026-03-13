use async_imap::Client as ImapClient;
use futures::TryStreamExt;
use tokio::net::TcpStream;
use tracing::{debug, info};

use crate::error::{MqError, Result};
use crate::oauth;

/// The TLS stream type used for IMAP connections.
/// Uses tokio-native-tls for native tokio AsyncRead/AsyncWrite compatibility.
pub type ImapTlsStream = tokio_native_tls::TlsStream<TcpStream>;

/// An authenticated IMAP session connected to Gmail.
pub struct ImapSession {
    session: async_imap::Session<ImapTlsStream>,
}

impl ImapSession {
    /// Connect and authenticate to Gmail IMAP using XOAUTH2.
    pub async fn connect(email: &str, access_token: &str) -> Result<Self> {
        let host = "imap.gmail.com";
        let port = 993;

        debug!(host, port, email, "Connecting to IMAP server");

        let tcp = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            TcpStream::connect((host, port)),
        )
        .await
        .map_err(|_| MqError::Other(anyhow::anyhow!("IMAP connection timed out after 30s")))?
        .map_err(|e| MqError::Imap(async_imap::error::Error::Io(e)))?;

        debug!("TCP connection established, starting TLS handshake");

        let tls_connector = native_tls::TlsConnector::new()
            .map_err(|e| MqError::Other(anyhow::anyhow!("TLS connector creation failed: {e}")))?;
        let tls_connector = tokio_native_tls::TlsConnector::from(tls_connector);

        let tls_stream = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tls_connector.connect(host, tcp),
        )
        .await
        .map_err(|_| MqError::Other(anyhow::anyhow!("TLS handshake timed out after 30s")))?
        .map_err(|e| MqError::Other(anyhow::anyhow!("TLS handshake failed: {e}")))?;

        debug!("TLS handshake complete, reading server greeting");

        let mut client = ImapClient::new(tls_stream);

        // Must read the server greeting before sending any commands.
        // The server sends "* OK Gimap ready..." upon connection.
        let _greeting = client
            .read_response()
            .await
            .map_err(|e| MqError::Other(anyhow::anyhow!("Failed to read IMAP greeting: {e}")))?
            .ok_or_else(|| MqError::Other(anyhow::anyhow!("IMAP server closed connection before greeting")))?;

        debug!("Server greeting received, authenticating");

        let auth_string = oauth::xoauth2_string(email, access_token);

        let session = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client.authenticate("XOAUTH2", ImapOAuth2 { auth_string }),
        )
        .await
        .map_err(|_| MqError::Other(anyhow::anyhow!("IMAP authentication timed out after 30s")))?
        .map_err(|(e, _)| e)?;

        info!(email, "IMAP authentication successful");
        Ok(Self { session })
    }

    /// List all mailboxes (folders/labels).
    pub async fn list_mailboxes(&mut self) -> Result<Vec<String>> {
        let mailboxes: Vec<_> = self
            .session
            .list(Some(""), Some("*"))
            .await?
            .try_collect()
            .await?;
        let names: Vec<String> = mailboxes.iter().map(|m| m.name().to_string()).collect();
        debug!(count = names.len(), "Listed mailboxes");
        Ok(names)
    }

    /// SELECT a mailbox, returning metadata including CONDSTORE info.
    pub async fn select(&mut self, mailbox: &str) -> Result<MailboxInfo> {
        let mbox = self.session.select(mailbox).await?;
        let info = MailboxInfo {
            exists: mbox.exists,
            uid_validity: mbox.uid_validity,
            highest_modseq: mbox.highest_modseq,
        };
        debug!(
            mailbox,
            exists = info.exists,
            uid_validity = ?info.uid_validity,
            highest_modseq = ?info.highest_modseq,
            "Selected mailbox"
        );
        Ok(info)
    }

    /// Fetch message headers for a range of UIDs.
    pub async fn fetch_headers(
        &mut self,
        uid_range: &str,
    ) -> Result<Vec<async_imap::types::Fetch>> {
        let fetched: Vec<_> = self
            .session
            .uid_fetch(
                uid_range,
                "(UID FLAGS ENVELOPE X-GM-MSGID X-GM-THRID X-GM-LABELS BODY.PEEK[HEADER.FIELDS (LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST)] BODYSTRUCTURE)",
            )
            .await?
            .try_collect()
            .await?;
        debug!(count = fetched.len(), uid_range, "Fetched headers");
        Ok(fetched)
    }

    /// Fetch the full body of a message by UID.
    pub async fn fetch_body(&mut self, uid: u32) -> Result<Option<Vec<u8>>> {
        let messages: Vec<_> = self
            .session
            .uid_fetch(uid.to_string(), "BODY.PEEK[]")
            .await?
            .try_collect()
            .await?;
        let body = messages.first().and_then(|m| m.body().map(|b| b.to_vec()));
        Ok(body)
    }

    /// Store flags on a message.
    pub async fn store_flags(&mut self, uid: u32, flags: &str, add: bool) -> Result<()> {
        let query = if add {
            format!("+FLAGS ({flags})")
        } else {
            format!("-FLAGS ({flags})")
        };
        let _responses: Vec<_> = self
            .session
            .uid_store(uid.to_string(), &query)
            .await?
            .try_collect()
            .await?;
        Ok(())
    }

    /// Move a message to another mailbox (Gmail MOVE extension).
    pub async fn move_message(&mut self, uid: u32, destination: &str) -> Result<()> {
        // Gmail supports MOVE. Fall back to COPY+DELETE if needed.
        self.session
            .uid_mv(uid.to_string(), destination)
            .await?;
        Ok(())
    }

    /// Run a raw IMAP search query.
    pub async fn search(&mut self, query: &str) -> Result<Vec<u32>> {
        let uids = self.session.uid_search(query).await?;
        let result: Vec<u32> = uids.into_iter().collect();
        Ok(result)
    }

    /// Close the session gracefully.
    pub async fn logout(mut self) -> Result<()> {
        self.session.logout().await?;
        Ok(())
    }

    /// Fetch Gmail-specific metadata (X-GM-MSGID, X-GM-THRID, X-GM-LABELS)
    /// using raw IMAP commands. async-imap's parser doesn't handle Gmail
    /// extensions, so we parse the raw response bytes ourselves.
    pub async fn fetch_gmail_metadata_raw(
        &mut self,
        uid_range: &str,
    ) -> Result<Vec<super::gmail_ext::GmailMetadata>> {
        use std::collections::HashMap;

        let tag = self
            .session
            .run_command(format!(
                "UID FETCH {uid_range} (UID X-GM-MSGID X-GM-THRID X-GM-LABELS)"
            ))
            .await?;

        let mut uid_map: HashMap<u32, super::gmail_ext::GmailMetadata> = HashMap::new();

        loop {
            match self.session.read_response().await {
                Ok(Some(resp)) => {
                    let line = format!("{:?}", resp.parsed());
                    // Check if this is the tagged OK response (end of FETCH)
                    if line.contains("Done(Ok") || line.contains(&format!("{:?}", tag)) {
                        break;
                    }
                    // Parse untagged FETCH responses from raw Debug output.
                    // The raw bytes contain lines like:
                    //   * 5 FETCH (UID 123 X-GM-MSGID 456 X-GM-THRID 789 X-GM-LABELS (...))
                    // Since imap-proto doesn't parse Gmail extensions, look at
                    // the raw bytes in the owner field via Debug.
                    let raw = format!("{:?}", resp);
                    // Log the first response to help diagnose parsing issues
                    if uid_map.is_empty() {
                        tracing::trace!(raw_sample = &raw[..raw.len().min(500)], "First Gmail metadata response");
                    }
                    if let Some(uid) = parse_uid_from_raw(&raw) {
                        let meta = uid_map
                            .entry(uid)
                            .or_insert_with(|| super::gmail_ext::GmailMetadata {
                                uid,
                                ..Default::default()
                            });
                        if let Some(msg_id) = parse_num_attr(&raw, "X-GM-MSGID") {
                            meta.gmail_msg_id = Some(msg_id);
                        }
                        if let Some(thrid) = parse_num_attr(&raw, "X-GM-THRID") {
                            meta.gmail_thread_id = Some(thrid);
                        }
                        let labels = parse_gm_labels_raw(&raw);
                        if !labels.is_empty() {
                            meta.labels = labels;
                        }
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        let with_thrid = uid_map.values().filter(|m| m.gmail_thread_id.is_some()).count();
        let result: Vec<_> = uid_map.into_values().collect();
        debug!(
            count = result.len(),
            with_thread_ids = with_thrid,
            uid_range,
            "Fetched Gmail metadata (raw)"
        );
        if with_thrid == 0 && !result.is_empty() {
            tracing::warn!(
                "No X-GM-THRID found in Gmail metadata — threading may not work. \
                 Run with RUST_LOG=trace to see raw response format."
            );
        }
        Ok(result)
    }

    /// Get a mutable reference to the underlying session (for sync/idle extensions).
    pub fn inner_mut(&mut self) -> &mut async_imap::Session<ImapTlsStream> {
        &mut self.session
    }

    /// Consume this wrapper and return the underlying async-imap session.
    ///
    /// Used by the IDLE handler which needs ownership of the session.
    pub fn into_inner(self) -> async_imap::Session<ImapTlsStream> {
        self.session
    }

    /// Reconstruct from an underlying async-imap session.
    ///
    /// Used after IDLE returns the session via `handle.done()`.
    pub fn from_inner(session: async_imap::Session<ImapTlsStream>) -> Self {
        Self { session }
    }
}

/// Mailbox metadata returned by SELECT, including CONDSTORE fields.
#[derive(Debug, Clone)]
pub struct MailboxInfo {
    pub exists: u32,
    pub uid_validity: Option<u32>,
    /// HIGHESTMODSEQ for CONDSTORE delta sync.
    pub highest_modseq: Option<u64>,
}

/// XOAUTH2 authenticator for async-imap.
struct ImapOAuth2 {
    auth_string: String,
}

impl async_imap::Authenticator for ImapOAuth2 {
    type Response = String;

    fn process(&mut self, _data: &[u8]) -> Self::Response {
        self.auth_string.clone()
    }
}

/// Parse UID from raw IMAP response debug output.
fn parse_uid_from_raw(raw: &str) -> Option<u32> {
    // Look for "UID " followed by digits in the raw response
    let marker = "UID ";
    let idx = raw.find(marker)?;
    let rest = &raw[idx + marker.len()..];
    let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

/// Parse a numeric attribute (X-GM-MSGID or X-GM-THRID) from raw bytes.
fn parse_num_attr(raw: &str, attr: &str) -> Option<u64> {
    let marker = format!("{attr} ");
    let idx = raw.find(&marker)?;
    let rest = &raw[idx + marker.len()..];
    let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

/// Parse X-GM-LABELS from raw IMAP response.
fn parse_gm_labels_raw(raw: &str) -> Vec<String> {
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
    super::gmail_ext::parse_label_list_pub(label_str)
}
