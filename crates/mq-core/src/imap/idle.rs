//! IMAP IDLE handler for real-time push notifications.
//!
//! After sync, enters IDLE on the selected mailbox. When the server
//! signals new messages (EXISTS), triggers an incremental sync.
//! Re-enters IDLE every 25 minutes (Gmail timeout is 29 min).

use std::time::Duration;

use async_imap::extensions::idle::IdleResponse;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::imap::client::ImapSession;

/// Events emitted by the IDLE watcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleEvent {
    /// The server reported new messages exist — trigger a sync.
    NewData,
    /// The IDLE timed out (25 min) — automatically re-entered.
    Timeout,
    /// The connection was lost.
    ConnectionLost,
}

/// IDLE timeout: 25 minutes (Gmail's server timeout is 29 min).
const IDLE_TIMEOUT: Duration = Duration::from_secs(25 * 60);

/// Run the IDLE loop on a mailbox, emitting events when changes occur.
///
/// This takes ownership of the `ImapSession`, enters IDLE, and loops:
/// - On new data → sends `IdleEvent::NewData` and returns the session
///   so the caller can run a sync and then call `idle_loop` again.
/// - On timeout → re-enters IDLE automatically.
/// - On error → sends `IdleEvent::ConnectionLost` and returns `None`.
///
/// The `cancel` receiver can be used to break out of the loop (e.g.
/// on app shutdown or when the user switches mailboxes).
pub async fn idle_loop(
    mut session: ImapSession,
    mailbox: &str,
    event_tx: mpsc::UnboundedSender<IdleEvent>,
    mut cancel: mpsc::Receiver<()>,
) -> Option<ImapSession> {
    // Make sure we have the mailbox selected
    if let Err(e) = session.select(mailbox).await {
        warn!(mailbox, "Failed to select mailbox for IDLE: {e}");
        let _ = event_tx.send(IdleEvent::ConnectionLost);
        return None;
    }

    info!(mailbox, "Entering IDLE loop");

    loop {
        // Take ownership of the inner async-imap session for IDLE
        let inner = session.into_inner();
        let mut idle_handle = inner.idle();

        // Send the IDLE command
        if let Err(e) = idle_handle.init().await {
            warn!("IDLE init failed: {e}");
            let _ = event_tx.send(IdleEvent::ConnectionLost);
            return None;
        }

        debug!(mailbox, "IDLE active, waiting for server events");

        // Wait for server response or timeout (25 min)
        let (idle_fut, _stop) = idle_handle.wait_with_timeout(IDLE_TIMEOUT);

        // Race between IDLE response and cancel signal
        let idle_result = tokio::select! {
            result = idle_fut => result,
            _ = cancel.recv() => {
                info!("IDLE cancelled by caller");
                // Try to cleanly exit IDLE
                if let Ok(inner_session) = idle_handle.done().await {
                    return Some(ImapSession::from_inner(inner_session));
                }
                return None;
            }
        };

        // End IDLE to get the session back
        let inner_session = match idle_handle.done().await {
            Ok(s) => s,
            Err(e) => {
                warn!("IDLE done failed: {e}");
                let _ = event_tx.send(IdleEvent::ConnectionLost);
                return None;
            }
        };

        session = ImapSession::from_inner(inner_session);

        match idle_result {
            Ok(IdleResponse::NewData(_)) => {
                info!(mailbox, "IDLE: server reported new data");
                let _ = event_tx.send(IdleEvent::NewData);
                // Return session so caller can sync, then re-enter IDLE
                return Some(session);
            }
            Ok(IdleResponse::Timeout) => {
                debug!(mailbox, "IDLE timed out, re-entering");
                let _ = event_tx.send(IdleEvent::Timeout);
                // Loop continues — re-enter IDLE
            }
            Ok(IdleResponse::ManualInterrupt) => {
                info!(mailbox, "IDLE manually interrupted");
                return Some(session);
            }
            Err(e) => {
                warn!(mailbox, "IDLE error: {e}");
                let _ = event_tx.send(IdleEvent::ConnectionLost);
                return None;
            }
        }
    }
}
