# Run the PhotoKit web app on http://127.0.0.1:7878 and open the browser.
Set-Location (Join-Path $PSScriptRoot "..")
cargo run --release -p photokit-cli -- serve --open @args
