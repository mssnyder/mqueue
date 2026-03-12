//! Network connectivity monitor via NetworkManager D-Bus.
//!
//! Watches the `StateChanged` signal from org.freedesktop.NetworkManager
//! and exposes connectivity state as a stream.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use tracing::{info, warn};
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
pub struct NetworkMonitor {
    state: Arc<AtomicU8>,
}

impl NetworkMonitor {
    /// Create a new network monitor. Falls back to assuming online
    /// if NetworkManager is unavailable.
    pub async fn new() -> Self {
        let state = Arc::new(AtomicU8::new(Connectivity::Online.to_u8()));

        match Self::get_initial_state().await {
            Ok(connectivity) => {
                info!(?connectivity, "Initial network state");
                state.store(connectivity.to_u8(), Ordering::SeqCst);
            }
            Err(e) => {
                warn!("Failed to connect to NetworkManager D-Bus: {e}. Assuming online.");
            }
        }

        Self { state }
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

    /// Whether we currently have internet connectivity.
    pub fn is_online(&self) -> bool {
        Connectivity::from_u8(self.state.load(Ordering::SeqCst)) == Connectivity::Online
    }

    /// Current connectivity state.
    pub fn connectivity(&self) -> Connectivity {
        Connectivity::from_u8(self.state.load(Ordering::SeqCst))
    }

    // TODO (Phase 6): Add a `connectivity_changes()` method that returns
    // a Stream by subscribing to the StateChanged D-Bus signal.
}
