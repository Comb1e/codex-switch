$ErrorActionPreference = 'Stop'

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Cargo was not found. Install Rust from https://rustup.rs and try again.'
}

$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
cargo install --path $ProjectRoot --force
if ($LASTEXITCODE -ne 0) {
    throw "cargo install failed with exit code $LASTEXITCODE"
}

$CargoBin = if ($env:CARGO_HOME) {
    Join-Path $env:CARGO_HOME 'bin'
} else {
    Join-Path $env:USERPROFILE '.cargo\bin'
}

$Installed = Join-Path $CargoBin 'cs.exe'
if (-not (Test-Path -LiteralPath $Installed)) {
    throw "Installation completed but cs.exe was not found at $Installed"
}

& $Installed --version
if ($env:PATH -notlike "*$CargoBin*") {
    Write-Host "Add $CargoBin to your PATH to run cs from any PowerShell session."
}
