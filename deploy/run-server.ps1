$ErrorActionPreference = 'Stop'

$secretFile = 'C:\burvy\secrets\ai-todo\jev-key.env'
if (-not $env:TYPESAFE_API_KEY -and (Test-Path $secretFile)) {
    $value = (Get-Content -Raw $secretFile).Trim()
    if ($value -match '^\s*TYPESAFE_API_KEY\s*=\s*(.*?)\s*$') {
        $value = $Matches[1].Trim().Trim('"').Trim("'")
    }
    if ($value) { $env:TYPESAFE_API_KEY = $value }
}
if (-not $env:PORT) { $env:PORT = '3160' }
if (-not $env:TZ_NAME) { $env:TZ_NAME = 'America/New_York' }

$server = Join-Path $PSScriptRoot 'dist\ai-todo\server.exe'
if (-not (Test-Path $server)) { throw "Missing $server. Run build-release.ps1 first." }
# The server serves ./static and writes ./data relative to its working directory.
Set-Location (Split-Path $server)
& $server
exit $LASTEXITCODE
