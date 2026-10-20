#!/usr/bin/env sh
# Build installers: .app + .dmg on macOS (use scripts/package.ps1 on Windows).
set -e
cd "$(dirname "$0")/../apps/desktop/src-tauri"
command -v cargo-tauri >/dev/null || cargo install tauri-cli --version "^2" --locked
cargo tauri build --bundles app,dmg "$@"
echo "Bundles are in target/release/bundle/"
