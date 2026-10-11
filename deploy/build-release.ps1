$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$stage = Join-Path $PSScriptRoot 'dist\ai-todo'

Write-Host 'Building server release...'
cargo build --release -p server --manifest-path (Join-Path $repoRoot 'Cargo.toml')

New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -Force (Join-Path $repoRoot 'target\release\server.exe') (Join-Path $stage 'server.exe')
$static = Join-Path $repoRoot 'static'
if (Test-Path $static) {
    Copy-Item -Recurse -Force $static (Join-Path $stage 'static')
}
Write-Host "Staged release at $stage"
