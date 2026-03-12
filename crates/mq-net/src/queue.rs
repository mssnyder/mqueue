//! Offline operation queue.
//!
//! When the network is offline, operations are serialized to the
//! offline_queue database table. On reconnection, they are replayed
//! in FIFO order.

// TODO: Implement in Phase 6.
