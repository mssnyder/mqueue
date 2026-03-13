//! Network connectivity monitor via NetworkManager D-Bus.
//!
//! Watches the `StateChanged` signal from org.freedesktop.NetworkManager
//! and exposes connectivity state as a stream.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use zbus::Connection;
use zbus::zvariant::OwnedValue;

/// Network connectivity state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connectivity {
    Online,
    Offline,
    /// Connected but limited (e.g., captive portal, local-only).
    Limited,
}

impl std::fmt::Display for Connectivity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Connectivity::Online => write!(f, "Online"),
            Connectivity::Offline => write!(f, "Offline"),
            Connectivity::Limited => write!(f, "Limited"),
        }
    }
}

impl From<u32> for Connectivity {
    fn from(nm_state: u32) -> Self {
        // NetworkManager states:
        // 70 = NM_STATE_CONNECTED_GLOBAL
        // 60 = NM_STATE_CONNECTED_SITE
        // 50 = NM_STATE_CONNECTED_LOCAL
        // Everything else = offline/disconnecting
        match nm_state {
            70 => Connectivity::Online,
            50 | 60 => Connectivity::Limited,
            _ => Connectivity::Offline,
        }
    }
}

impl Connectivity {
    fn to_u8(self) -> u8 {
        match self {
            Connectivity::Online => 2,
            Connectivity::Limited => 1,
            Connectivity::Offline => 0,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            2 => Connectivity::Online,
            1 => Connectivity::Limited,
            _ => Connectivity::Offline,
        }
    }
}

/// Monitors network connectivity via NetworkManager D-Bus.
///
/// Subscribes to the `StateChanged` D-Bus signal so consumers can react
/// to connectivity transitions (e.g. trigger resync on reconnect).
pub struct NetworkMonitor {
    state: Arc<AtomicU8>,
    sender: broadcast::Sender<Connectivity>,
}

impl NetworkMonitor {
    /// Create a new network monitor and start watching for D-Bus signals.
    ///
    /// Falls back to assuming online if NetworkManager is unavailable.
    pub async fn new() -> Self {
        let state = Arc::new(AtomicU8::new(Connectivity::Online.to_u8()));
        let (sender, _) = broadcast::channel(16);

        match Self::get_initial_state().await {
            Ok(connectivity) => {
                info!(?connectivity, "Initial network state");
                state.store(connectivity.to_u8(), Ordering::SeqCst);
            }
            Err(e) => {
                warn!("Failed to connect to NetworkManager D-Bus: {e}. Assuming online.");
            }
        }

        let monitor = Self { state, sender };

        // Spawn background task to watch D-Bus StateChanged signal
        let state_ref = monitor.state.clone();
        let sender_ref = monitor.sender.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::watch_state_changes(state_ref, sender_ref).await {
                warn!("NetworkManager D-Bus watch ended: {e}");
            }
        });

        monitor
    }

    /// Subscribe to connectivity change notifications.
    ///
    /// Returns a receiver that yields the new `Connectivity` state
    /// each time NetworkManager reports a transition.
    pub fn subscribe(&self) -> broadcast::Receiver<Connectivity> {
        self.sender.subscribe()
    }

    /// Whether we currently have internet connectivity.
    pub fn is_online(&self) -> bool {
        Connectivity::from_u8(self.state.load(Ordering::SeqCst)) == Connectivity::Online
    }

    /// Current connectivity state.
    pub fn connectivity(&self) -> Connectivity {
        Connectivity::from_u8(self.state.load(Ordering::SeqCst))
    }

    async fn get_initial_state() -> anyhow::Result<Connectivity> {
        let connection = Connection::system().await?;
        let proxy = zbus::fdo::PropertiesProxy::builder(&connection)
            .destination("org.freedesktop.NetworkManager")?
            .path("/org/freedesktop/NetworkManager")?
            .build()
            .await?;

        let iface_name = zbus::names::InterfaceName::from_static_str_unchecked(
            "org.freedesktop.NetworkManager",
        );
        let state: OwnedValue = proxy.get(iface_name, "State").await?;
        let nm_state: u32 = state.try_into()?;
        Ok(Connectivity::from(nm_state))
    }

    /// Watch for NetworkManager StateChanged signals on D-Bus and
    /// update the internal state + broadcast to subscribers.
    async fn watch_state_changes(
        state: Arc<AtomicU8>,
        sender: broadcast::Sender<Connectivity>,
    ) -> anyhow::Result<()> {
        use futures::StreamExt;

        let connection = Connection::system().await?;

        // Subscribe to the StateChanged signal
        let proxy = zbus::fdo::PropertiesProxy::builder(&connection)
            .destination("org.freedesktop.NetworkManager")?
            .path("/org/freedesktop/NetworkManager")?
            .build()
            .await?;

        let mut stream = proxy.receive_properties_changed().await?;
        info!("Listening for NetworkManager state changes");

        while let Some(signal) = stream.next().await {
            let args = match signal.args() {
                Ok(a) => a,
                Err(e) => {
                    debug!("Failed to parse PropertiesChanged args: {e}");
                    continue;
                }
            };

            // Check if "State" property changed
            if let Some(state_val) = args.changed_properties().get("State") {
                let nm_state: u32 = match state_val.try_into() {
                    Ok(v) => v,
                    Err(e) => {
                        debug!("Failed to parse NM state value: {e}");
                        continue;
                    }
                };

                let connectivity = Connectivity::from(nm_state);
                let old = Connectivity::from_u8(state.load(Ordering::SeqCst));

                if old != connectivity {
                    info!(%old, %connectivity, "Network state changed");
                    state.store(connectivity.to_u8(), Ordering::SeqCst);
                    // Ignore send error — just means no active subscribers
                    let _ = sender.send(connectivity);
                }
            }
        }

        Ok(())
    }
}
