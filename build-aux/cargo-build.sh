#!/usr/bin/env bash
# Build the mq-mail binary with Cargo and copy it to the Meson output path.
# Called by meson.build as a custom_target command.
set -euo pipefail

OUTPUT="$1"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}"

# Use release profile by default; override with PROFILE=debug for dev builds.
PROFILE="${PROFILE:-release}"
PROFILE_DIR="$PROFILE"
if [ "$PROFILE" = "dev" ]; then
    PROFILE_DIR="debug"
fi

cargo build --profile "$PROFILE" -p mq-app

cp "${CARGO_TARGET_DIR}/${PROFILE_DIR}/mq-mail" "$OUTPUT"
