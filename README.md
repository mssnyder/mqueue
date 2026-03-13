# m'Queue

A privacy-focused native Linux email client for Gmail, built with Rust, GTK4, and libadwaita.

## Features

- **Gmail-only** via IMAP with Google OAuth2 (XOAUTH2)
- **Privacy by default** -- remote images blocked, tracking pixels removed, link trackers stripped
- **One-click unsubscribe** (RFC 8058 / RFC 2369)
- **Offline-first** -- SQLite cache with offline operation queue
- **Multi-account** with unified inbox view
- **Adaptive layout** -- works at any window size, including tiling WMs
- **CONDSTORE delta sync** for efficient incremental updates
- **Desktop notifications** via GIO
- **Nix-native** -- flake with NixOS and home-manager modules

## Screenshots

*Coming soon*

## Installation

### Nix (recommended)

Add to your flake inputs:

```nix
inputs.mq-mail.url = "github:mssnyder/mqueue";
```

#### Home Manager

```nix
{ inputs, ... }:
{
  imports = [ inputs.mq-mail.homeManagerModules.default ];

  programs.mq-mail = {
    enable = true;
    settings = {
      oauth = {
        # Plaintext (for testing):
        # clientId = "your-client-id";
        # clientSecret = "your-client-secret";

        # File-based (for sops-nix / agenix):
        clientIdFile = "/run/user/1000/secrets/mqueue/googleClientId";
        clientSecretFile = "/run/user/1000/secrets/mqueue/googleSecretKey";
      };
    };
  };
}
```

#### NixOS

```nix
{ inputs, ... }:
{
  imports = [ inputs.mq-mail.nixosModules.default ];

  programs.mq-mail = {
    enable = true;
    settings.oauth.clientIdFile = "/run/secrets/mqueue/googleClientId";
    settings.oauth.clientSecretFile = "/run/secrets/mqueue/googleSecretKey";
  };
}
```

### From source

Requires Rust 1.92+, GTK4, libadwaita 1.5+, WebKitGTK 6, and SQLite.

```sh
nix develop  # sets up the full dev environment
cargo build --release
```

## Google OAuth Setup

m'Queue requires you to create your own Google Cloud project:

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project
3. Enable the **Gmail API**
4. Create OAuth 2.0 credentials (Desktop application)
5. Add `http://127.0.0.1` as an authorized redirect URI
6. Provide the client ID and secret via your Nix config or `~/.config/mq-mail/config.toml`:

```toml
[oauth]
client_id = "your-client-id.apps.googleusercontent.com"
client_secret = "your-client-secret"
```

## Configuration

Base config is loaded from `~/.config/mq-mail/config.toml` (managed by Nix if using the module). User preferences set in the app are saved separately to `~/.config/mq-mail/preferences.toml` and merged on top, so Nix-managed settings are never overwritten.

Tokens are stored securely in GNOME Keyring via the [oo7](https://crates.io/crates/oo7) crate.

## Architecture

```
crates/
  mq-core/   # Domain logic: IMAP, SMTP, OAuth, email parsing, privacy
  mq-db/     # SQLite database layer (sqlx)
  mq-net/    # Network awareness (NetworkManager D-Bus) + offline queue
  mq-app/    # GTK4/libadwaita UI
```

## Privacy

- **Remote images** are blocked by default (per-sender allowlist available)
- **Tracking pixels** (1x1 images, known tracker domains) are unconditionally removed
- **Link tracking parameters** (`utm_*`, `fbclid`, `gclid`, etc.) are stripped on click
- **Unsubscribe** via RFC 8058 one-click POST or mailto fallback

## License

[GPL-3.0-or-later](https://www.gnu.org/licenses/gpl-3.0.html)
