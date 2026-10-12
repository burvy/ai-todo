# Build and run Rambl locally, then open it in the browser.
#   .\run-local.ps1            persistent data in data\ai-todo.db
#   .\run-local.ps1 -Fresh     throwaway in-memory data (clean demo)
#   .\run-local.ps1 -Port 9000 -LogCapture   (log Jev judgments)
param(
    [int]$Port = 8080,
    [switch]$Fresh,
    [switch]$LogCapture
)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# Jev key: rambl.env (TYPESAFE_API_KEY=...) is loaded by the server itself; fall back to jev-api-key.env.
if (-not $env:TYPESAFE_API_KEY -and -not (Test-Path 'rambl.env') -and (Test-Path 'jev-api-key.env')) {
    $value = (Get-Content -Raw 'jev-api-key.env').Trim()
    if ($value -match '^\s*TYPESAFE_API_KEY\s*=\s*(.*?)\s*$') { $value = $Matches[1].Trim().Trim('"').Trim("'") }
    if ($value) { $env:TYPESAFE_API_KEY = $value }
}
if (-not $env:TYPESAFE_API_KEY -and -not (Test-Path 'rambl.env')) {
    Write-Warning 'No Jev key found (rambl.env or jev-api-key.env). Capture falls back to one task per line.'
}

$env:PORT = "$Port"
if (-not $env:RAMBL_DEFAULT_TZ) { $env:RAMBL_DEFAULT_TZ = 'America/New_York' }
if ($Fresh) { $env:DATA_PATH = ':memory:' }
if ($LogCapture) { $env:CAPTURE_DEBUG = '1' }

cargo build --release -p rambl-server
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$url = "http://localhost:$Port"
Write-Host "ai-todo on $url  (Ctrl+C to stop)"
Start-Job { param($u) Start-Sleep 2; Start-Process $u } -ArgumentList $url | Out-Null
& .\target\release\rambl-server.exe
exit $LASTEXITCODE
