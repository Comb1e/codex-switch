$ErrorActionPreference = 'Stop'

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Cargo was not found. Install Rust from https://rustup.rs and try again.'
}

$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$CargoBin = if ($env:CARGO_HOME) {
    Join-Path $env:CARGO_HOME 'bin'
} else {
    Join-Path $env:USERPROFILE '.cargo\bin'
}

$Installed = Join-Path $CargoBin 'cs.exe'
$Supervisor = Join-Path $CargoBin 'cs-relay.exe'

# Complete the expensive work before interrupting the installed service.
cargo build --release --locked --bins --manifest-path (Join-Path $ProjectRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed with exit code $LASTEXITCODE"
}

$PreviousProfile = $null
if (Test-Path -LiteralPath $Installed) {
    $PreviousProfile = & $Installed current
    if ($LASTEXITCODE -ne 0 -or $PreviousProfile -eq 'unmanaged') {
        $PreviousProfile = $null
    }
}

# A running Windows executable cannot be overwritten. Stop only our installed
# supervisor and relay workers, in that order, so recovery cannot race the copy.
foreach ($Executable in @($Supervisor, $Installed)) {
    $InstalledProcesses = Get-CimInstance Win32_Process -Filter "Name = 'cs.exe' OR Name = 'cs-relay.exe'" |
        Where-Object {
            $_.ExecutablePath -eq $Executable -and
            ($Executable -eq $Supervisor -or $_.CommandLine -match '\s__relay(?:\s|$)')
        }
    foreach ($InstalledProcess in $InstalledProcesses) {
        $Process = Get-Process -Id $InstalledProcess.ProcessId -ErrorAction SilentlyContinue
        if ($Process) {
            Stop-Process -InputObject $Process -ErrorAction Stop
            if (-not $Process.WaitForExit(10000)) {
                throw "Installed relay process $($Process.Id) did not stop."
            }
        }
    }
}

try {
    cargo install --path $ProjectRoot --locked --force
    if ($LASTEXITCODE -ne 0) {
        throw "cargo install failed with exit code $LASTEXITCODE"
    }
} catch {
    # Restore the previous working relay if replacement failed.
    if ($PreviousProfile -and (Test-Path -LiteralPath $Installed)) {
        & $Installed $PreviousProfile
    }
    throw
}

if (-not (Test-Path -LiteralPath $Installed)) {
    throw "Installation completed but cs.exe was not found at $Installed"
}

& $Installed --version
if ($LASTEXITCODE -ne 0) {
    throw "cs --version failed with exit code $LASTEXITCODE"
}

# Migrate existing startup entries and start the saved profile with the new binary.
& $Installed __relay-install
if ($LASTEXITCODE -ne 0) {
    throw "Relay startup migration failed with exit code $LASTEXITCODE"
}
if ($env:PATH -notlike "*$CargoBin*") {
    Write-Host "Add $CargoBin to your PATH to run cs from any PowerShell session."
}
