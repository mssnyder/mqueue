use async_imap::Client as ImapClient;
use async_native_tls::TlsConnector;
use futures::TryStreamExt;
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{debug, info};

use crate::error::{MqError, Result};
use crate::oauth;

/// The TLS stream type used for IMAP connections.
/// tokio TcpStream is wrapped with tokio_util::compat to provide futures_io traits
/// that async-imap and async-native-tls require.
type ImapTlsStream = async_native_tls::TlsStream<tokio_util::compat::Compat<TcpStream>>;

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

        let tcp = TcpStream::connect((host, port)).await.map_err(|e| {
            MqError::Imap(async_imap::error::Error::Io(e))
        })?;

        // Wrap tokio TcpStream with compat layer for futures_io traits
        let tcp_compat = tcp.compat();

        let tls = TlsConnector::new();
        let tls_stream = tls.connect(host, tcp_compat).await.map_err(|e| {
            MqError::Other(anyhow::anyhow!("TLS handshake failed: {e}"))
        })?;

        let client = ImapClient::new(tls_stream);

        let auth_string = oauth::xoauth2_string(email, access_token);

        let session = client
            .authenticate("XOAUTH2", ImapOAuth2 { auth_string })
            .await
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

    /// SELECT a mailbox, returning metadata.
    pub async fn select(&mut self, mailbox: &str) -> Result<MailboxInfo> {
        let mbox = self.session.select(mailbox).await?;
        let info = MailboxInfo {
            exists: mbox.exists,
            uid_validity: mbox.uid_validity,
        };
        debug!(mailbox, exists = info.exists, "Selected mailbox");
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
                "(UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST)] BODYSTRUCTURE)",
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

    /// Get a mutable reference to the underlying session (for sync/idle extensions).
    pub fn inner_mut(&mut self) -> &mut async_imap::Session<ImapTlsStream> {
        &mut self.session
    }
}

/// Basic mailbox metadata returned by SELECT.
#[derive(Debug, Clone)]
pub struct MailboxInfo {
    pub exists: u32,
    pub uid_validity: Option<u32>,
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
