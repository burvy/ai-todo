$ErrorActionPreference = 'Stop'

# Builds only Rambl's server and stages a clean folder to copy to the server:
#   server.exe, static\, .env.example
# Double-click server.exe to run it; it reads .env next to itself.
$repoRoot = Split-Path -Parent $PSScriptRoot
$stage = Join-Path $PSScriptRoot 'dist\rambl-release'

Write-Host 'Building Rambl (server only)...'
cargo build --release -p server --manifest-path (Join-Path $repoRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

# A fresh folder every time, so no local test data (data\) or old files ride along.
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -Force (Join-Path $repoRoot 'target\release\server.exe') (Join-Path $stage 'server.exe')
Copy-Item -Recurse -Force (Join-Path $repoRoot 'static') (Join-Path $stage 'static')
@'
# Copy to .env (next to server.exe) on the server and fill in the key.
TYPESAFE_API_KEY=
PORT=3160
RAMBL_DATA=data/rambl.db
RAMBL_DEFAULT_TZ=America/New_York
'@ | Set-Content -Encoding utf8 (Join-Path $stage '.env.example')

Write-Host "Staged release at $stage"
if (-not $env:NO_OPEN) { Start-Process explorer.exe $stage }
