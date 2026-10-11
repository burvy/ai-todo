$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$stage = Join-Path $PSScriptRoot 'dist\ai-todo'

Write-Host 'Building server release...'
cargo build --release -p server --manifest-path (Join-Path $repoRoot 'Cargo.toml')

New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -Force (Join-Path $repoRoot 'target\release\server.exe') (Join-Path $stage 'server.exe')
$static = Join-Path $repoRoot 'static'
if (Test-Path $static) {
    # Replace the staged copy; copying onto an existing folder would nest it as static\static.
    $stagedStatic = Join-Path $stage 'static'
    if (Test-Path $stagedStatic) { Remove-Item -Recurse -Force $stagedStatic }
    Copy-Item -Recurse -Force $static $stagedStatic
}
Write-Host "Staged release at $stage"
