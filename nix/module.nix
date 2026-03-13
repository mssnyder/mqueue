self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.mq-mail;
  inherit (lib) mkEnableOption mkOption types mkIf;

  # Generate config.toml from the Nix options
  configFile = pkgs.writeText "mq-mail-config.toml" (lib.generators.toINI {} (
    lib.filterAttrsRecursive (n: v: v != null) {
      oauth = {
        client_id = cfg.settings.oauth.clientId;
        client_secret = cfg.settings.oauth.clientSecret;
        client_id_file = cfg.settings.oauth.clientIdFile;
        client_secret_file = cfg.settings.oauth.clientSecretFile;
      };
      privacy = {
        block_remote_images = cfg.settings.privacy.blockRemoteImages;
        detect_tracking_pixels = cfg.settings.privacy.detectTrackingPixels;
        strip_tracking_params = cfg.settings.privacy.stripTrackingParams;
      };
      compose = {
        default_signature = cfg.settings.compose.defaultSignature;
        reply_position = cfg.settings.compose.replyPosition;
      };
      logging = {
        file_enabled = cfg.settings.logging.fileEnabled;
        file_path = cfg.settings.logging.filePath;
        journald_enabled = cfg.settings.logging.journaldEnabled;
        level = cfg.settings.logging.level;
      };
      cache = {
        retention_days = cfg.settings.cache.retentionDays;
      };
      appearance = {
        theme = cfg.settings.appearance.theme;
      };
      notifications = {
        enabled = cfg.settings.notifications.enabled;
        sound = cfg.settings.notifications.sound;
      };
    }
  ));

  tomlFormat = pkgs.formats.toml { };

  settingsToml = tomlFormat.generate "mq-mail-config.toml" (
    lib.filterAttrsRecursive (n: v: v != null) {
      oauth = lib.filterAttrs (n: v: v != null) {
        client_id = cfg.settings.oauth.clientId;
        client_secret = cfg.settings.oauth.clientSecret;
        client_id_file =
          if cfg.settings.oauth.clientIdFile != null
          then toString cfg.settings.oauth.clientIdFile
          else null;
        client_secret_file =
          if cfg.settings.oauth.clientSecretFile != null
          then toString cfg.settings.oauth.clientSecretFile
          else null;
      };
      privacy = {
        block_remote_images = cfg.settings.privacy.blockRemoteImages;
        detect_tracking_pixels = cfg.settings.privacy.detectTrackingPixels;
        strip_tracking_params = cfg.settings.privacy.stripTrackingParams;
      };
      compose = {
        default_signature = cfg.settings.compose.defaultSignature;
        reply_position = cfg.settings.compose.replyPosition;
      };
      logging = lib.filterAttrs (n: v: v != null) {
        file_enabled = cfg.settings.logging.fileEnabled;
        file_path =
          if cfg.settings.logging.filePath != null
          then toString cfg.settings.logging.filePath
          else null;
        journald_enabled = cfg.settings.logging.journaldEnabled;
        level = cfg.settings.logging.level;
      };
      cache = {
        retention_days = cfg.settings.cache.retentionDays;
      };
      appearance = {
        theme = cfg.settings.appearance.theme;
      };
      notifications = {
        enabled = cfg.settings.notifications.enabled;
        sound = cfg.settings.notifications.sound;
      };
    }
  );
in
{
  options.programs.mq-mail = {
    enable = mkEnableOption "m'Queue email client";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.system}.default;
      description = "The mq-mail package to use.";
    };

    settings = {
      oauth = {
        clientId = mkOption {
          type = types.nullOr types.str;
          default = null;
          description = "Google OAuth2 client ID (plaintext).";
        };
        clientSecret = mkOption {
          type = types.nullOr types.str;
          default = null;
          description = "Google OAuth2 client secret (plaintext).";
        };
        clientIdFile = mkOption {
          type = types.nullOr types.path;
          default = null;
          description = "Path to file containing Google OAuth2 client ID (for sops-nix/agenix).";
        };
        clientSecretFile = mkOption {
          type = types.nullOr types.path;
          default = null;
          description = "Path to file containing Google OAuth2 client secret (for sops-nix/agenix).";
        };
      };

      privacy = {
        blockRemoteImages = mkOption {
          type = types.bool;
          default = true;
          description = "Block remote images in emails by default.";
        };
        detectTrackingPixels = mkOption {
          type = types.bool;
          default = true;
          description = "Detect and remove tracking pixels.";
        };
        stripTrackingParams = mkOption {
          type = types.bool;
          default = true;
          description = "Strip UTM and other tracking parameters from links.";
        };
      };

      compose = {
        defaultSignature = mkOption {
          type = types.str;
          default = "";
          description = "Default email signature.";
        };
        replyPosition = mkOption {
          type = types.enum [ "above" "below" ];
          default = "above";
          description = "Where to place the cursor when replying (above or below quoted text).";
        };
      };

      logging = {
        fileEnabled = mkOption {
          type = types.bool;
          default = false;
          description = "Enable logging to a file.";
        };
        filePath = mkOption {
          type = types.nullOr types.path;
          default = null;
          description = "Path to log file.";
        };
        journaldEnabled = mkOption {
          type = types.bool;
          default = false;
          description = "Enable logging to the systemd journal.";
        };
        level = mkOption {
          type = types.enum [ "error" "warn" "info" "debug" "trace" ];
          default = "info";
          description = "Log level filter.";
        };
      };

      cache = {
        retentionDays = mkOption {
          type = types.int;
          default = 90;
          description = "Number of days to keep cached message bodies.";
        };
      };

      appearance = {
        theme = mkOption {
          type = types.enum [ "system" "light" "dark" ];
          default = "system";
          description = "UI theme preference.";
        };
      };

      notifications = {
        enabled = mkOption {
          type = types.bool;
          default = true;
          description = "Enable desktop notifications for new mail.";
        };
        sound = mkOption {
          type = types.bool;
          default = false;
          description = "Play a sound on new mail notification.";
        };
      };
    };
  };

  config = mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile."mq-mail/config.toml".source = settingsToml;
  };
}
