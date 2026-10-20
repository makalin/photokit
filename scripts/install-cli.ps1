# Install the `photokit` command into %USERPROFILE%\.cargo\bin.
Set-Location (Join-Path $PSScriptRoot "..")
cargo install --path crates/photokit-cli --locked
