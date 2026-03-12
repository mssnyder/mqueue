//! CONDSTORE-based IMAP sync engine.
//!
//! Uses HIGHESTMODSEQ to efficiently detect changes since the last sync,
//! avoiding full mailbox re-fetches.

// Tracing will be used when sync engine is implemented in Phase 2.
#[allow(unused_imports)]
use tracing::{debug, info, warn};

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
pub enum SyncResult {
    /// No changes since last sync.
    NoChanges,
    /// Incremental changes detected.
    Incremental {
        new_message_uids: Vec<u32>,
        changed_flag_uids: Vec<u32>,
    },
    /// UIDVALIDITY changed; full resync required.
    FullResyncRequired,
}

// TODO: Implement the full sync engine in Phase 2.
// This module will:
// 1. SELECT mailbox (CONDSTORE)
// 2. Compare UIDVALIDITY with stored value
// 3. Compare HIGHESTMODSEQ and fetch deltas
// 4. Fetch headers for new messages
// 5. Update local sync state
