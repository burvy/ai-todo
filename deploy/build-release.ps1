$ErrorActionPreference = 'Stop'

# Builds only Rambl's server and stages a clean folder to copy to the server:
#   server.exe, .env.example
# Double-click server.exe to run it; it reads .env next to itself.
$repoRoot = Split-Path -Parent $PSScriptRoot
$stage = Join-Path $PSScriptRoot 'dist\rambl-release'

Write-Host 'Building the web bundle...'
Push-Location (Join-Path $repoRoot 'web')
trunk build --release
if ($LASTEXITCODE -ne 0) { Pop-Location; throw 'trunk build failed' }
Pop-Location

Write-Host 'Building Rambl server...'
cargo build --release -p server --manifest-path (Join-Path $repoRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

# A fresh folder every time, so no local test data (data\) or old files ride along.
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -Force (Join-Path $repoRoot 'target\release\server.exe') (Join-Path $stage 'server.exe')
@'
TYPESAFE_API_KEY=
PORT=3160
RAMBL_DEFAULT_TZ=America/New_York
GOOGLE_CLIENT_ID=
'@ | Set-Content -Encoding utf8 (Join-Path $stage '.env.example')

Write-Host "Staged release at $stage"
if (-not $env:NO_OPEN) { Start-Process explorer.exe $stage }
Write-Host 'Release steps: copy server.exe and .env.example to the server, create .env, then double-click server.exe.'
