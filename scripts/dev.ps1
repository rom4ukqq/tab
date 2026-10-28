param([switch]$Build)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (Test-Path '.tools\cargo\bin\cargo.exe') {
    $env:CARGO_HOME = Join-Path $PWD '.tools\cargo'
    $env:RUSTUP_HOME = Join-Path $PWD '.tools\rustup'
    $env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
}
Push-Location 'apps\desktop'
try { if ($Build) { npm.cmd run tauri -- build --bundles nsis } else { npm.cmd run tauri -- dev } } finally { Pop-Location }
