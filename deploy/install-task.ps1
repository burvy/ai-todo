$ErrorActionPreference = 'Stop'

$taskName = 'ai-todo-server'
$runScript = Join-Path $PSScriptRoot 'run-server.ps1'
$workingDirectory = Join-Path $PSScriptRoot 'dist\ai-todo'
$action = New-ScheduledTaskAction -Execute 'PowerShell.exe' -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$runScript`"" -WorkingDirectory $workingDirectory
$trigger = New-ScheduledTaskTrigger -AtStartup
$settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
$principal = New-ScheduledTaskPrincipal -UserId 'NT AUTHORITY\LocalService' -LogonType ServiceAccount -RunLevel Limited
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null

if (-not (Get-NetFirewallRule -DisplayName 'ai-todo-server TCP 3160' -ErrorAction SilentlyContinue)) {
    New-NetFirewallRule -DisplayName 'ai-todo-server TCP 3160' -Direction Inbound -Protocol TCP -LocalPort 3160 -Action Allow -Profile Any | Out-Null
}
Write-Host "Installed scheduled task $taskName and firewall rule for TCP 3160."
