# Run ON THE SERVER after copying a new release folder over (e.g. via RustDesk).
#   .\update-server.ps1 -From C:\Users\Burvy\Desktop\ai-todo-new
# Replaces server.exe and static\ in deploy\dist\ai-todo, keeps data\ (tasks, calendar),
# then starts the server in this window. Stop the old server window (Ctrl+C) first.
param(
    [Parameter(Mandatory = $true)][string]$From
)
$ErrorActionPreference = 'Stop'

$target = Join-Path $PSScriptRoot 'dist\ai-todo'
$newExe = Join-Path $From 'server.exe'
$newStatic = Join-Path $From 'static'
if (-not (Test-Path $newExe)) { throw "No server.exe in $From" }
if (-not (Test-Path $newStatic)) { throw "No static folder in $From" }

if (Get-Process server -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$target*" }) {
    throw 'The old server is still running. Press Ctrl+C in its window, then run this again.'
}

New-Item -ItemType Directory -Force -Path $target | Out-Null
Copy-Item -Force $newExe (Join-Path $target 'server.exe')
$static = Join-Path $target 'static'
if (Test-Path $static) { Remove-Item -Recurse -Force $static }
Copy-Item -Recurse -Force $newStatic $static
Write-Host "Updated $target (data\ kept). Starting the server..."

& (Join-Path $PSScriptRoot 'run-server.ps1')
