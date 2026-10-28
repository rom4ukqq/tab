param()
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (Test-Path '.tools\cargo\bin\cargo.exe') {
    $env:CARGO_HOME = Join-Path $PWD '.tools\cargo'
    $env:RUSTUP_HOME = Join-Path $PWD '.tools\rustup'
    $env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
}
cargo test --locked -p hub-core
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Push-Location 'apps\desktop'
try { npm.cmd run build; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE } } finally { Pop-Location }
