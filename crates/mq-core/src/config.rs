use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{MqError, Result};

/// Application configuration loaded from config.toml.
///
/// Sources checked in priority order:
/// 1. `$XDG_CONFIG_HOME/mq-mail/config.toml`
/// 2. Defaults
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub oauth: OAuthConfig,
    pub privacy: PrivacyConfig,
    pub compose: ComposeConfig,
    pub logging: LoggingConfig,
    pub cache: CacheConfig,
    pub appearance: AppearanceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OAuthConfig {
    /// Google OAuth2 client ID (plaintext).
    pub client_id: Option<String>,
    /// Google OAuth2 client secret (plaintext).
    pub client_secret: Option<String>,
    /// Path to a file containing the client ID (for sops-nix / agenix).
    pub client_id_file: Option<PathBuf>,
    /// Path to a file containing the client secret (for sops-nix / agenix).
    pub client_secret_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacyConfig {
    pub block_remote_images: bool,
    pub detect_tracking_pixels: bool,
    pub strip_tracking_params: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ComposeConfig {
    pub default_signature: String,
    pub reply_position: ReplyPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub file_enabled: bool,
    pub file_path: Option<PathBuf>,
    pub journald_enabled: bool,
    pub level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    pub retention_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub theme: Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplyPosition {
    Above,
    Below,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

// --- Defaults ---

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            oauth: OAuthConfig::default(),
            privacy: PrivacyConfig::default(),
            compose: ComposeConfig::default(),
            logging: LoggingConfig::default(),
            cache: CacheConfig::default(),
            appearance: AppearanceConfig::default(),
        }
    }
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            client_id: None,
            client_secret: None,
            client_id_file: None,
            client_secret_file: None,
        }
    }
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            block_remote_images: true,
            detect_tracking_pixels: true,
            strip_tracking_params: true,
        }
    }
}

impl Default for ComposeConfig {
    fn default() -> Self {
        Self {
            default_signature: String::new(),
            reply_position: ReplyPosition::Above,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            file_enabled: false,
            file_path: None,
            journald_enabled: false,
            level: "info".into(),
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            retention_days: 90,
        }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: Theme::System,
        }
    }
}

impl Default for ReplyPosition {
    fn default() -> Self {
        Self::Above
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::System
    }
}

// --- Loading ---

impl AppConfig {
    /// Returns the path to the config directory: `$XDG_CONFIG_HOME/mq-mail/`.
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("mq-mail")
    }

    /// Returns the path to the config file: `$XDG_CONFIG_HOME/mq-mail/config.toml`.
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Returns the path to the data directory: `$XDG_DATA_HOME/mq-mail/`.
    pub fn data_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("~/.local/share"))
            .join("mq-mail")
    }

    /// Load config from the standard config path, falling back to defaults.
    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            Self::load_from(&path)
        } else {
            Ok(Self::default())
        }
    }

    /// Load config from a specific file path.
    pub fn load_from(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path).map_err(|e| {
            MqError::Config(format!("Failed to read config file {}: {e}", path.display()))
        })?;
        let config: AppConfig = toml::from_str(&contents).map_err(|e| {
            MqError::Config(format!(
                "Failed to parse config file {}: {e}",
                path.display()
            ))
        })?;
        Ok(config)
    }

    /// Save the current config to the standard config path.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        let dir = Self::config_dir();
        fs::create_dir_all(&dir).map_err(|e| {
            MqError::Config(format!(
                "Failed to create config directory {}: {e}",
                dir.display()
            ))
        })?;
        let contents = toml::to_string_pretty(self)
            .map_err(|e| MqError::Config(format!("Failed to serialize config: {e}")))?;
        fs::write(&path, contents).map_err(|e| {
            MqError::Config(format!(
                "Failed to write config file {}: {e}",
                path.display()
            ))
        })?;
        Ok(())
    }

    /// Resolve the OAuth client ID, checking file-based sources first.
    pub fn resolve_client_id(&self) -> Result<Option<String>> {
        Self::resolve_secret(&self.oauth.client_id_file, &self.oauth.client_id, "client_id")
    }

    /// Resolve the OAuth client secret, checking file-based sources first.
    pub fn resolve_client_secret(&self) -> Result<Option<String>> {
        Self::resolve_secret(
            &self.oauth.client_secret_file,
            &self.oauth.client_secret,
            "client_secret",
        )
    }

    /// Resolve a secret value: file path takes precedence over inline value.
    fn resolve_secret(
        file_path: &Option<PathBuf>,
        inline: &Option<String>,
        name: &str,
    ) -> Result<Option<String>> {
        if let Some(path) = file_path {
            let value = fs::read_to_string(path)
                .map_err(|e| {
                    MqError::Config(format!(
                        "Failed to read {name} from file {}: {e}",
                        path.display()
                    ))
                })?
                .trim()
                .to_string();
            if value.is_empty() {
                return Err(MqError::Config(format!(
                    "{name} file {} is empty",
                    path.display()
                )));
            }
            Ok(Some(value))
        } else {
            Ok(inline.clone())
        }
    }
}
