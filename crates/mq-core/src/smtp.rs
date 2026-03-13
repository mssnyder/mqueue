//! SMTP sending via lettre with XOAUTH2 authentication.
//!
//! Sends email through smtp.gmail.com:587 (STARTTLS).

use lettre::message::header::ContentType;
use lettre::message::{Mailbox, MessageBuilder, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use tracing::{debug, info};

use crate::error::{MqError, Result};
use crate::oauth;

/// Parameters for composing and sending an email.
#[derive(Debug, Clone)]
pub struct OutgoingEmail {
    pub from_email: String,
    pub from_name: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

/// Send an email via Gmail SMTP with XOAUTH2 authentication.
pub async fn send_email(email: &OutgoingEmail, access_token: &str) -> Result<()> {
    let xoauth2 = oauth::xoauth2_string(&email.from_email, access_token);

    let tls_params = TlsParameters::new("smtp.gmail.com".to_string())
        .map_err(|e| MqError::Other(anyhow::anyhow!("TLS setup error: {e}")))?;

    let transport: AsyncSmtpTransport<Tokio1Executor> =
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay("smtp.gmail.com")
            .map_err(|e| MqError::Other(anyhow::anyhow!("SMTP relay error: {e}")))?
            .port(587)
            .tls(Tls::Required(tls_params))
            .credentials(Credentials::new(email.from_email.clone(), xoauth2))
            .authentication(vec![Mechanism::Xoauth2])
            .build();

    let message = build_message(email)?;

    info!(to = ?email.to, subject = %email.subject, "Sending email");

    transport.send(message).await?;

    info!("Email sent successfully");
    Ok(())
}

/// Build a lettre Message from our OutgoingEmail parameters.
fn build_message(email: &OutgoingEmail) -> Result<Message> {
    let from_mailbox: Mailbox = if let Some(ref name) = email.from_name {
        format!("{name} <{}>", email.from_email)
    } else {
        email.from_email.clone()
    }
    .parse()
    .map_err(|e| MqError::Other(anyhow::anyhow!("Invalid From address: {e}")))?;

    let mut builder: MessageBuilder = Message::builder().from(from_mailbox);

    for addr in &email.to {
        let mailbox: Mailbox = addr
            .parse()
            .map_err(|e| MqError::Other(anyhow::anyhow!("Invalid To address '{addr}': {e}")))?;
        builder = builder.to(mailbox);
    }

    for addr in &email.cc {
        let mailbox: Mailbox = addr
            .parse()
            .map_err(|e| MqError::Other(anyhow::anyhow!("Invalid Cc address '{addr}': {e}")))?;
        builder = builder.cc(mailbox);
    }

    for addr in &email.bcc {
        let mailbox: Mailbox = addr
            .parse()
            .map_err(|e| MqError::Other(anyhow::anyhow!("Invalid Bcc address '{addr}': {e}")))?;
        builder = builder.bcc(mailbox);
    }

    builder = builder.subject(&email.subject);

    if let Some(ref msg_id) = email.in_reply_to {
        builder = builder.in_reply_to(msg_id.clone());
    }
    if let Some(ref refs) = email.references {
        builder = builder.references(refs.clone());
    }

    let message = if let Some(ref html) = email.body_html {
        builder
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(email.body_text.clone()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(html.clone()),
                    ),
            )
            .map_err(|e| MqError::Other(anyhow::anyhow!("Failed to build message: {e}")))?
    } else {
        builder
            .body(email.body_text.clone())
            .map_err(|e| MqError::Other(anyhow::anyhow!("Failed to build message: {e}")))?
    };

    debug!("Built email message");
    Ok(message)
}
