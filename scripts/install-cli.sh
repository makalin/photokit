#!/usr/bin/env sh
# Install the `photokit` command into ~/.cargo/bin.
set -e
cd "$(dirname "$0")/.."
cargo install --path crates/photokit-cli --locked
