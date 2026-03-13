{
  description = "m'Queue — A privacy-focused native Linux Gmail client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        # Use a recent stable Rust toolchain (gtk4-rs 0.11 requires Rust >= 1.92)
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Common native build inputs needed for linking
        nativeBuildInputs = with pkgs; [
          pkg-config
          wrapGAppsHook4
        ];

        # Libraries needed at build time and runtime
        buildInputs = with pkgs; [
          gtk4
          libadwaita
          webkitgtk_6_0
          sqlite
          openssl
          glib
          dbus
          gdk-pixbuf
          pango
          cairo
          graphene
        ];

        # Filter source to include Rust files + data directory
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            (craneLib.filterCargoSources path type)
            || (builtins.match ".*\\.desktop$" path != null)
            || (builtins.match ".*\\.metainfo\\.xml$" path != null)
            || (builtins.match ".*\\.svg$" path != null)
            || (builtins.match ".*\\.css$" path != null)
            || (builtins.match ".*/data/.*" path != null);
        };

        commonArgs = {
          inherit src nativeBuildInputs buildInputs;
          strictDeps = true;
        };

        # Build just the cargo dependencies for caching
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        appId = "com.mqmail.MqMail";

        # Build the full package
        mq-mail = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;

          # Run tests as part of the build
          doCheck = true;

          # Install desktop integration files after cargo build
          postInstall = ''
            # Desktop file
            install -Dm644 data/${appId}.desktop \
              $out/share/applications/${appId}.desktop

            # AppStream metainfo
            install -Dm644 data/${appId}.metainfo.xml \
              $out/share/metainfo/${appId}.metainfo.xml

            # Application icon
            install -Dm644 data/${appId}.svg \
              $out/share/icons/hicolor/scalable/apps/${appId}.svg

            # CSS stylesheet
            install -Dm644 data/resources/style.css \
              $out/share/mq-mail/style.css

            # Bundled fallback icons (used when the user's icon theme
            # doesn't include symbolic variants)
            cp -r data/icons $out/share/mq-mail/icons
          '';

          meta = with pkgs.lib; {
            description = "m'Queue — A privacy-focused native Linux Gmail client";
            license = licenses.gpl3Plus;
            platforms = platforms.linux;
            mainProgram = "mq-mail";
          };
        });
      in
      {
        packages = {
          default = mq-mail;
          mq-mail = mq-mail;
        };

        devShells.default = craneLib.devShell {
          # Inherit all build inputs from the main package
          inputsFrom = [ mq-mail ];

          packages = with pkgs; [
            # Rust development tools (rust-analyzer included via toolchain overlay)
            cargo-watch
            cargo-nextest

            # Debugging
            gdb

            # GTK4 development
            gtk4.dev
            libadwaita.dev

            # D-Bus debugging
            dbus
          ];

          shellHook = ''
            echo "m'Queue dev shell ready (Rust $(rustc --version)). Run 'cargo build' to build."
          '';
        };

        checks = {
          inherit mq-mail;

          mq-mail-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

          mq-mail-fmt = craneLib.cargoFmt {
            inherit src;
          };
        };
      }
    ) // {
      overlays.default = final: prev: {
        mq-mail = self.packages.${final.system}.default;
      };

      nixosModules.default = import ./nix/module.nix self;
      homeManagerModules.default = import ./nix/module.nix self;
    };
}
