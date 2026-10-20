#!/usr/bin/env sh
# Run the desktop app in development mode.
set -e
cd "$(dirname "$0")/.."
cargo run -p photokit-desktop
