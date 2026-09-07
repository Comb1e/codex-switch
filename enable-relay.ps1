param(
    [string]$Profile = 'transit',
    [string]$Upstream = 'http://154.201.72.176:8080/'
)

$ErrorActionPreference = 'Stop'

$runningApps = Get-Process -ErrorAction SilentlyContinue |
    Where-Object { $_.ProcessName -match '^(Codex|ChatGPT)$' }
if ($runningApps) {
    throw 'Close the Codex and ChatGPT desktop apps, then run this script again.'
}

$switchCommand = Get-Command cs -ErrorAction SilentlyContinue
if (-not $switchCommand) {
    throw 'cs was not found on PATH. Run .\install.ps1 first.'
}

$codexHome = if ($env:CODEX_HOME) {
    $env:CODEX_HOME
} else {
    Join-Path $env:USERPROFILE '.codex'
}
$authPath = Join-Path $codexHome 'auth.json'
if (-not (Test-Path -LiteralPath $authPath)) {
    throw "Codex authentication file not found: $authPath"
}

$auth = Get-Content -Raw -LiteralPath $authPath | ConvertFrom-Json
$relayKey = [string]$auth.OPENAI_API_KEY
if ([string]::IsNullOrWhiteSpace($relayKey)) {
    throw 'OPENAI_API_KEY is missing from auth.json. Add the profile with cs add --apikey-stdin instead.'
}

$addArgs = @(
    'add',
    '--name', $Profile,
    '--baseurl', $Upstream,
    '--apikey-stdin',
    '--force'
)
$relayKey | & $switchCommand.Source @addArgs
if ($LASTEXITCODE -ne 0) {
    throw "cs add failed with exit code $LASTEXITCODE"
}

& $switchCommand.Source $Profile
if ($LASTEXITCODE -ne 0) {
    throw "cs activation failed with exit code $LASTEXITCODE"
}

Write-Host ''
Write-Host 'Relay configuration is ready.'
Write-Host "auth.json was not changed: $authPath"
Write-Host 'Now replace auth.json with the ChatGPT auth file, then restart Codex Desktop.'
if ($Upstream -match '^http://') {
    Write-Warning 'The upstream uses plain HTTP. API keys, prompts, and responses are not encrypted in transit.'
}
