param(
  [string]$TaskName = "DataCollector",
  [string]$RepoPath = (Resolve-Path "$PSScriptRoot\..").Path,
  [string]$CondaEnv = "DATA_C",
  [string]$ConfigPath = "configs\\config.yaml",
  [ValidateSet("rust","python")] [string]$CollectorImpl = "rust",
  [ValidateSet("Logon","Startup")] [string]$Trigger = "Logon",
  [int]$RestartCount = 3,
  [int]$RestartMinutes = 1,
  [switch]$RunAsHighest
)

$ErrorActionPreference = "Stop"

$runScript = Join-Path $RepoPath "scripts\\run_core.ps1"
if (-not (Test-Path $runScript)) {
  throw "run_core.ps1 not found at $runScript"
}

$arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$runScript`" -RepoPath `"$RepoPath`" -CondaEnv `"$CondaEnv`" -ConfigPath `"$ConfigPath`" -CollectorImpl $CollectorImpl"

$action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument $arguments
if ($Trigger -eq "Startup") {
  $trigger = New-ScheduledTaskTrigger -AtStartup
} else {
  $trigger = New-ScheduledTaskTrigger -AtLogOn
}

$settings = New-ScheduledTaskSettingsSet `
  -RestartCount $RestartCount `
  -RestartInterval (New-TimeSpan -Minutes $RestartMinutes) `
  -StartWhenAvailable `
  -MultipleInstances IgnoreNew `
  -ExecutionTimeLimit (New-TimeSpan -Days 365) `
  -AllowStartIfOnBatteries `
  -DontStopIfGoingOnBatteries

if ($RunAsHighest) {
  $principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -RunLevel Highest
  Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null
} else {
  Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
}

Write-Host "Task registered: $TaskName"
