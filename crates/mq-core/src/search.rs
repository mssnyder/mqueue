//! Search abstraction.
//!
//! Provides server-side Gmail search via X-GM-RAW. Local FTS5 search
//! is handled directly through `mq_db::queries::messages::search_fts`.
//! The app layer routes between server and local search based on connectivity.

pub use crate::imap::gmail_ext::search_gmail;
