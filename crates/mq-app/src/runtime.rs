//! Bridge between the tokio async runtime and the GTK main loop.
//!
//! Tokio runs on a background thread. Results are sent back to the
//! GTK main thread via tokio::sync::oneshot + glib::spawn_future_local.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Get or initialize the shared tokio runtime.
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        Runtime::new().expect("Failed to create tokio runtime")
    })
}

/// Spawn an async task on the tokio runtime and invoke `callback` on the
/// GTK main thread with the result.
pub fn spawn_async<F, T, C>(future: F, callback: C)
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    C: FnOnce(T) + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();

    runtime().spawn(async move {
        let result = future.await;
        let _ = tx.send(result);
    });

    glib::spawn_future_local(async move {
        if let Ok(result) = rx.await {
            callback(result);
        }
    });
}
