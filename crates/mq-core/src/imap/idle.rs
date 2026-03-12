//! IMAP IDLE handler for real-time push notifications.
//!
//! After sync, enters IDLE on the selected mailbox. When the server
//! signals new messages (EXISTS), triggers an incremental sync.
//! Re-enters IDLE every 25 minutes (Gmail timeout is 29 min).

// TODO: Implement in Phase 6.
