//! Offline operation queue.
//!
//! When the network is offline, operations (flag changes, label mutations,
//! message moves, sends) are serialized to the `offline_queue` database table.
//! On reconnection, they are replayed in FIFO order.

use std::sync::Arc;

use mq_db::SqlitePool;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

/// Types of operations that can be queued for offline replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OfflineOp {
    /// Add or remove flags on a message.
    StoreFlags {
        uid: u32,
        mailbox: String,
        flags: String,
        add: bool,
    },
    /// Move a message to another mailbox.
    MoveMessage {
        uid: u32,
        from_mailbox: String,
        to_mailbox: String,
    },
    /// Delete a message (move to Trash).
    DeleteMessage {
        uid: u32,
        mailbox: String,
    },
}

/// Manages the offline operation queue.
///
/// Operations are enqueued when the network is unavailable and replayed
/// in FIFO order when connectivity is restored.
pub struct OfflineQueue {
    pool: Arc<SqlitePool>,
}

impl OfflineQueue {
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    /// Enqueue an operation for later replay.
    pub async fn enqueue(
        &self,
        account_id: i64,
        op: OfflineOp,
    ) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        let op_type = match &op {
            OfflineOp::StoreFlags { .. } => "store_flags",
            OfflineOp::MoveMessage { .. } => "move_message",
            OfflineOp::DeleteMessage { .. } => "delete_message",
        };
        let payload = serde_json::to_string(&op).unwrap_or_default();

        let id =
            mq_db::queries::offline_queue::enqueue_op(&self.pool, account_id, op_type, &payload)
                .await?;
        debug!(id, %op_type, "Enqueued offline operation");
        Ok(id)
    }

    /// Number of pending operations for a given account.
    pub async fn pending_count(
        &self,
        account_id: i64,
    ) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(mq_db::queries::offline_queue::pending_op_count(&self.pool, account_id).await?)
    }

    /// Total pending operations across all accounts.
    pub async fn total_pending_count(
        &self,
    ) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        Ok(mq_db::queries::offline_queue::total_pending_count(&self.pool).await?)
    }

    /// Replay all pending operations for an account.
    ///
    /// The `executor` callback is called for each operation. If it returns
    /// `Ok(())`, the operation is marked complete. On error, it's marked
    /// failed with the error message.
    ///
    /// Returns the number of successfully replayed operations.
    pub async fn replay<F, Fut>(
        &self,
        account_id: i64,
        executor: F,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>>
    where
        F: Fn(OfflineOp) -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        let ops = mq_db::queries::offline_queue::get_pending_ops(&self.pool, account_id).await?;

        if ops.is_empty() {
            return Ok(0);
        }

        info!(count = ops.len(), account_id, "Replaying offline operations");
        let mut success_count = 0;

        for op in ops {
            let parsed: Result<OfflineOp, _> = serde_json::from_str(&op.payload);
            match parsed {
                Ok(offline_op) => {
                    let op_desc = format!("{:?}", offline_op);
                    match executor(offline_op).await {
                        Ok(()) => {
                            mq_db::queries::offline_queue::mark_op_complete(&self.pool, op.id)
                                .await?;
                            debug!(id = op.id, %op_desc, "Replayed operation successfully");
                            success_count += 1;
                        }
                        Err(e) => {
                            warn!(id = op.id, %op_desc, error = %e, "Operation replay failed");
                            mq_db::queries::offline_queue::mark_op_failed(&self.pool, op.id, &e)
                                .await?;
                        }
                    }
                }
                Err(e) => {
                    error!(id = op.id, payload = %op.payload, "Failed to parse queued operation: {e}");
                    mq_db::queries::offline_queue::mark_op_failed(
                        &self.pool,
                        op.id,
                        &format!("Parse error: {e}"),
                    )
                    .await?;
                }
            }
        }

        info!(success_count, account_id, "Offline queue replay complete");
        Ok(success_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offline_op_serialization() {
        let op = OfflineOp::StoreFlags {
            uid: 42,
            mailbox: "INBOX".to_string(),
            flags: "\\Seen".to_string(),
            add: true,
        };
        let json = serde_json::to_string(&op).unwrap();
        let parsed: OfflineOp = serde_json::from_str(&json).unwrap();
        match parsed {
            OfflineOp::StoreFlags {
                uid,
                mailbox,
                flags,
                add,
            } => {
                assert_eq!(uid, 42);
                assert_eq!(mailbox, "INBOX");
                assert_eq!(flags, "\\Seen");
                assert!(add);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_move_message_serialization() {
        let op = OfflineOp::MoveMessage {
            uid: 10,
            from_mailbox: "INBOX".to_string(),
            to_mailbox: "[Gmail]/Trash".to_string(),
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains("move_message"));
        let parsed: OfflineOp = serde_json::from_str(&json).unwrap();
        match parsed {
            OfflineOp::MoveMessage {
                uid,
                from_mailbox,
                to_mailbox,
            } => {
                assert_eq!(uid, 10);
                assert_eq!(from_mailbox, "INBOX");
                assert_eq!(to_mailbox, "[Gmail]/Trash");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_delete_message_serialization() {
        let op = OfflineOp::DeleteMessage {
            uid: 99,
            mailbox: "INBOX".to_string(),
        };
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains("delete_message"));
        let _: OfflineOp = serde_json::from_str(&json).unwrap();
    }
}
