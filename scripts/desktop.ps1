# Run the desktop app in development mode.
Set-Location (Join-Path $PSScriptRoot "..")
cargo run -p photokit-desktop
