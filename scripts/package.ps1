# Build Windows installers (.msi and setup .exe).
Set-Location (Join-Path $PSScriptRoot "..\apps\desktop\src-tauri")
if (-not (Get-Command cargo-tauri -ErrorAction SilentlyContinue)) { cargo install tauri-cli --version "^2" --locked }
cargo tauri build --bundles msi,nsis @args
Write-Host "Installers are in target\release\bundle\"
