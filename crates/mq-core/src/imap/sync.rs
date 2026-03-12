//! CONDSTORE-based IMAP sync engine.
//!
//! Uses HIGHESTMODSEQ to efficiently detect changes since the last sync,
//! avoiding full mailbox re-fetches.

use futures::TryStreamExt;
use tracing::{debug, info, warn};

use crate::email::{Email, MessageFlags};
use crate::error::Result;
use crate::imap::client::ImapSession;
use crate::imap::parse;

/// Sync state for a single mailbox, persisted in the database.
#[derive(Debug, Clone)]
pub struct SyncState {
    pub mailbox: String,
    pub uid_validity: u32,
    pub highest_modseq: u64,
    pub highest_uid: u32,
}

/// Result of a sync operation.
#[derive(Debug)]
pub struct SyncOutcome {
    /// Updated sync state to persist.
    pub new_state: SyncState,
    /// Newly fetched messages (envelope + flags).
    pub new_messages: Vec<Email>,
    /// Flag updates for existing messages.
    pub flag_updates: Vec<FlagUpdate>,
    /// UIDs that no longer exist on the server (expunged).
    pub expunged_uids: Vec<u32>,
}

/// A flag change for an existing message.
#[derive(Debug, Clone)]
pub struct FlagUpdate {
    pub uid: u32,
    pub flags: MessageFlags,
}

/// Sync a single mailbox, comparing against previously stored state.
///
/// `known_uids` should contain all UIDs currently stored locally for this
/// mailbox, used to detect expunged messages.
pub async fn sync_mailbox(
    session: &mut ImapSession,
    mailbox: &str,
    prev_state: Option<&SyncState>,
    known_uids: &[u32],
) -> Result<SyncOutcome> {
    info!(mailbox, "Starting mailbox sync");

    // SELECT the mailbox to get current state (including CONDSTORE info)
    let mbox_info = session.select(mailbox).await?;
    let uid_validity = mbox_info.uid_validity.unwrap_or(0);
    let highest_modseq = mbox_info.highest_modseq.unwrap_or(0);

    debug!(
        mailbox,
        exists = mbox_info.exists,
        uid_validity,
        highest_modseq,
        "Mailbox state"
    );

    // Check if UIDVALIDITY changed (requires full resync)
    if let Some(prev) = prev_state {
        if prev.uid_validity != uid_validity {
            warn!(
                mailbox,
                old = prev.uid_validity,
                new = uid_validity,
                "UIDVALIDITY changed, full resync required"
            );
            return full_sync(session, mailbox, uid_validity, highest_modseq).await;
        }

        // If HIGHESTMODSEQ hasn't changed, nothing to do
        if prev.highest_modseq == highest_modseq && mbox_info.exists > 0 {
            debug!(mailbox, "No changes (HIGHESTMODSEQ unchanged)");
            return Ok(SyncOutcome {
                new_state: SyncState {
                    mailbox: mailbox.to_string(),
                    uid_validity,
                    highest_modseq,
                    highest_uid: prev.highest_uid,
                },
                new_messages: vec![],
                flag_updates: vec![],
                expunged_uids: vec![],
            });
        }
    }

    // If no messages exist in mailbox
    if mbox_info.exists == 0 {
        let expunged = known_uids.to_vec();
        return Ok(SyncOutcome {
            new_state: SyncState {
                mailbox: mailbox.to_string(),
                uid_validity,
                highest_modseq,
                highest_uid: 0,
            },
            new_messages: vec![],
            flag_updates: vec![],
            expunged_uids: expunged,
        });
    }

    let prev_highest_uid = prev_state.map(|s| s.highest_uid).unwrap_or(0);

    // Fetch new messages (UID > prev_highest_uid)
    let new_messages = if prev_highest_uid > 0 {
        let range = format!("{}:*", prev_highest_uid + 1);
        fetch_new_messages(session, &range, prev_highest_uid).await?
    } else {
        // First sync: fetch all messages (limited to most recent)
        fetch_new_messages(session, "1:*", 0).await?
    };

    let new_highest_uid = new_messages
        .iter()
        .map(|e| e.uid)
        .max()
        .unwrap_or(prev_highest_uid);

    // Fetch flag updates for existing messages
    let flag_updates = if !known_uids.is_empty() && prev_state.is_some() {
        fetch_flag_updates(session, known_uids).await?
    } else {
        vec![]
    };

    // Detect expunged messages by comparing server UIDs with known local UIDs
    let expunged_uids = if !known_uids.is_empty() {
        detect_expunged(session, known_uids).await?
    } else {
        vec![]
    };

    let new_count = new_messages.len();
    let flag_count = flag_updates.len();
    let expunge_count = expunged_uids.len();
    info!(
        mailbox,
        new_count, flag_count, expunge_count, "Sync complete"
    );

    Ok(SyncOutcome {
        new_state: SyncState {
            mailbox: mailbox.to_string(),
            uid_validity,
            highest_modseq,
            highest_uid: new_highest_uid,
        },
        new_messages,
        flag_updates,
        expunged_uids,
    })
}

/// Full resync: fetch all message envelopes. Called when UIDVALIDITY changes.
async fn full_sync(
    session: &mut ImapSession,
    mailbox: &str,
    uid_validity: u32,
    highest_modseq: u64,
) -> Result<SyncOutcome> {
    info!(mailbox, "Performing full resync");
    let messages = fetch_new_messages(session, "1:*", 0).await?;
    let highest_uid = messages.iter().map(|e| e.uid).max().unwrap_or(0);

    Ok(SyncOutcome {
        new_state: SyncState {
            mailbox: mailbox.to_string(),
            uid_validity,
            highest_modseq,
            highest_uid,
        },
        new_messages: messages,
        flag_updates: vec![],
        expunged_uids: vec![], // Caller should clear all local messages for this mailbox
    })
}

/// Fetch envelopes + flags for messages in the given UID range.
async fn fetch_new_messages(
    session: &mut ImapSession,
    uid_range: &str,
    exclude_below: u32,
) -> Result<Vec<Email>> {
    let fetches = session.fetch_headers(uid_range).await?;
    let mut messages = Vec::with_capacity(fetches.len());

    for fetch in &fetches {
        if let Some(email) = parse::parse_fetch(fetch) {
            // IMAP may return the boundary UID even when using uid+1:*
            if email.uid > exclude_below {
                messages.push(email);
            }
        }
    }

    debug!(count = messages.len(), uid_range, "Fetched new messages");
    Ok(messages)
}

/// Fetch current flags for a set of known UIDs.
async fn fetch_flag_updates(
    session: &mut ImapSession,
    known_uids: &[u32],
) -> Result<Vec<FlagUpdate>> {
    if known_uids.is_empty() {
        return Ok(vec![]);
    }

    // Build UID set string (e.g. "1,2,3,5,8")
    let uid_set = uids_to_set(known_uids);

    let fetches: Vec<_> = session
        .inner_mut()
        .uid_fetch(&uid_set, "FLAGS")
        .await?
        .try_collect()
        .await?;

    let mut updates = Vec::new();
    for fetch in &fetches {
        if let Some((uid, flags)) = parse::parse_flags(fetch) {
            updates.push(FlagUpdate { uid, flags });
        }
    }

    debug!(count = updates.len(), "Fetched flag updates");
    Ok(updates)
}

/// Detect which known UIDs no longer exist on the server.
async fn detect_expunged(
    session: &mut ImapSession,
    known_uids: &[u32],
) -> Result<Vec<u32>> {
    // Search for all UIDs in the mailbox
    let server_uids = session.search("ALL").await?;
    let server_set: std::collections::HashSet<u32> =
        server_uids.into_iter().collect();

    let expunged: Vec<u32> = known_uids
        .iter()
        .filter(|uid| !server_set.contains(uid))
        .copied()
        .collect();

    if !expunged.is_empty() {
        debug!(count = expunged.len(), "Detected expunged messages");
    }

    Ok(expunged)
}

/// Convert a slice of UIDs to an IMAP UID set string.
///
/// Groups consecutive UIDs into ranges for efficiency:
/// `[1,2,3,5,8,9,10]` → `"1:3,5,8:10"`
fn uids_to_set(uids: &[u32]) -> String {
    if uids.is_empty() {
        return String::new();
    }

    let mut sorted: Vec<u32> = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut parts = Vec::new();
    let mut range_start = sorted[0];
    let mut range_end = sorted[0];

    for &uid in &sorted[1..] {
        if uid == range_end + 1 {
            range_end = uid;
        } else {
            if range_start == range_end {
                parts.push(range_start.to_string());
            } else {
                parts.push(format!("{range_start}:{range_end}"));
            }
            range_start = uid;
            range_end = uid;
        }
    }

    if range_start == range_end {
        parts.push(range_start.to_string());
    } else {
        parts.push(format!("{range_start}:{range_end}"));
    }

    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uids_to_set_empty() {
        assert_eq!(uids_to_set(&[]), "");
    }

    #[test]
    fn test_uids_to_set_single() {
        assert_eq!(uids_to_set(&[42]), "42");
    }

    #[test]
    fn test_uids_to_set_consecutive() {
        assert_eq!(uids_to_set(&[1, 2, 3, 4, 5]), "1:5");
    }

    #[test]
    fn test_uids_to_set_mixed() {
        assert_eq!(uids_to_set(&[1, 2, 3, 5, 8, 9, 10]), "1:3,5,8:10");
    }

    #[test]
    fn test_uids_to_set_gaps() {
        assert_eq!(uids_to_set(&[1, 3, 5, 7]), "1,3,5,7");
    }

    #[test]
    fn test_uids_to_set_unsorted() {
        assert_eq!(uids_to_set(&[5, 1, 3, 2, 4]), "1:5");
    }
}
