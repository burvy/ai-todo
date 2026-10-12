$ErrorActionPreference = 'Stop'

# Builds Rambl end to end and stages a clean folder to copy to the server:
#   server.exe, dist\ (the web app), .env.example
# The web app is the Leptos front end in web\, built by Trunk into web\dist\ and
# staged as dist\ next to the exe, which is where the server looks for it.
# Double-click server.exe to run it; it reads .env next to itself.
$repoRoot = Split-Path -Parent $PSScriptRoot
$stage = Join-Path $PSScriptRoot 'dist\rambl-release'
$webDir = Join-Path $repoRoot 'web'

# The web build first: if the front end will not compile there is no point
# spending minutes on a server binary we cannot ship alongside it.
Write-Host 'Building the web app (trunk, release)...'
$trunk = Get-Command trunk -ErrorAction SilentlyContinue
if (-not $trunk) {
    throw ("trunk was not found on PATH. It builds the web app into web\dist. " +
           "Install it with 'cargo install trunk', or 'cargo install --locked trunk', " +
           "then run this script again.")
}
Push-Location $webDir
try {
    # trunk and cargo both report progress on stderr. Under
    # $ErrorActionPreference='Stop' that becomes a terminating error even on a
    # clean build, so the preference is relaxed around each call.
    #
    # Success is read from $LASTEXITCODE, so the command must not be piped:
    # routing through Out-Host clobbers it, and $? is unreliable here because
    # the pipeline result reflects the stderr record, not the exit code.
    $ErrorActionPreference = 'Continue'
    trunk build --release
    $exit = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($exit -ne 0) { throw "trunk build failed (exit $exit)" }
} finally {
    Pop-Location
}

$webDist = Join-Path $webDir 'dist'
if (-not (Test-Path (Join-Path $webDist 'index.html'))) {
    throw "trunk build finished but web\dist\index.html is missing; nothing to stage."
}

Write-Host 'Building the server...'
$ErrorActionPreference = 'Continue'
cargo build --release -p server --manifest-path (Join-Path $repoRoot 'Cargo.toml')
$exit = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
if ($exit -ne 0) { throw "cargo build failed (exit $exit)" }

# A fresh folder every time, so no local test data (data\) or old files ride along.
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -Force (Join-Path $repoRoot 'target\release\server.exe') (Join-Path $stage 'server.exe')
# As dist\, which is the first place the server looks; v1's static\ is only a
# fallback for builds made before the web app existed.
Copy-Item -Recurse -Force $webDist (Join-Path $stage 'dist')
@'
# Copy to .env (next to server.exe) on the server and fill in the key.
TYPESAFE_API_KEY=
PORT=3160
TZ_NAME=America/New_York
'@ | Set-Content -Encoding utf8 (Join-Path $stage '.env.example')

Write-Host "Staged release at $stage"
Write-Host 'It holds server.exe, dist\ (the web app) and .env.example.'
if (-not $env:NO_OPEN) { Start-Process explorer.exe $stage }
