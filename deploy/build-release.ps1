$ErrorActionPreference = 'Continue'
# PowerShell 5.1: native stderr + 'Stop' terminates; 'Continue' lets $LASTEXITCODE drive failure.

# Builds Rambl's server and stages a clean folder to copy to the server:
#   rambl-server.exe, rambl.env.example
# Double-click rambl-server.exe to run it; it reads rambl.env next to itself.
$repoRoot = Split-Path -Parent $PSScriptRoot
$stage = Join-Path $PSScriptRoot 'dist\rambl-release'

Write-Host 'Building the web bundle...'
Push-Location (Join-Path $repoRoot 'web')
trunk build --release
if ($LASTEXITCODE -ne 0) { Pop-Location; throw 'trunk build failed' }
Pop-Location

Write-Host 'Building Rambl server...'
cargo build --release -p rambl-server --manifest-path (Join-Path $repoRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

# A fresh folder every time, so no local test data (data\) or old files ride along.
if (Test-Path $stage) { Remove-Item -Recurse -Force -ErrorAction Stop $stage }
New-Item -ItemType Directory -Force -ErrorAction Stop -Path $stage | Out-Null
Copy-Item -Force -ErrorAction Stop (Join-Path $repoRoot 'target\release\rambl-server.exe') (Join-Path $stage 'rambl-server.exe')
@'
TYPESAFE_API_KEY=
RAMBL_PORT=3160
RAMBL_DEFAULT_TZ=America/New_York
RAMBL_ADMIN_EMAILS=boss@example.com
# Proxies whose CF-Connecting-IP names the real client. Leave empty when the tunnel
# is on this machine: loopback is trusted already. A comma list of addresses or
# networks, e.g. 10.0.0.1, 10.0.0.0/8
RAMBL_TRUSTED_PROXIES=
GOOGLE_CLIENT_ID=138340425604-qagdhrfblj66i5s9i9icq07lb32sd2g2.apps.googleusercontent.com
'@ | Set-Content -Encoding utf8 -ErrorAction Stop (Join-Path $stage 'rambl.env.example')

Write-Host "Staged release at $stage"
if (-not $env:NO_OPEN) { Start-Process explorer.exe $stage }
Write-Host 'Release steps: copy rambl-server.exe and rambl.env.example to the server, create rambl.env, then double-click rambl-server.exe.'
