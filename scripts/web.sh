#!/usr/bin/env sh
# Run the PhotoKit web app on http://127.0.0.1:7878 and open the browser.
set -e
cd "$(dirname "$0")/.."
cargo run --release -p photokit-cli -- serve --open "$@"
