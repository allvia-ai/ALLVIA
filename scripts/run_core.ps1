param(
  [string]$RepoPath = (Resolve-Path "$PSScriptRoot\..").Path,
  [string]$CondaEnv = "DATA_C",
  [string]$ConfigPath = "configs\\config.yaml",
  [ValidateSet("rust","python")] [string]$CollectorImpl = "rust",
  [switch]$NoConda
)

$ErrorActionPreference = "Stop"

Set-Location $RepoPath

$resolvedConfig = $ConfigPath
if (-not (Test-Path $resolvedConfig)) {
  $resolvedConfig = Join-Path $RepoPath $ConfigPath
}
if (-not (Test-Path $resolvedConfig)) {
  throw "Config not found: $resolvedConfig"
}

if ($CollectorImpl -eq "python") {
  $pythonArgs = @("-m", "collector.main", "--config", $resolvedConfig)
  if (-not $NoConda) {
    $conda = Get-Command conda -ErrorAction SilentlyContinue
    if ($conda) {
      & $conda.Path run -n $CondaEnv python @pythonArgs
      exit $LASTEXITCODE
    }
  }

  & python @pythonArgs
  exit $LASTEXITCODE
}

$manifestPath = Join-Path $RepoPath "core\\Cargo.toml"
$binaryPath = Join-Path $RepoPath "core\\target\\debug\\collector_rs.exe"

if (-not (Test-Path $binaryPath)) {
  & cargo build --manifest-path $manifestPath --bin collector_rs
  if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
  }
}

# Respect explicit override if already provided.
if (-not $env:STEER_DB_PATH) {
  $dbLine = Get-Content $resolvedConfig |
    Where-Object { $_ -match '^\s*db_path\s*:' } |
    Select-Object -First 1

  if ($dbLine) {
    $dbRaw = ($dbLine -replace '^\s*db_path\s*:\s*', '').Trim()
    $dbRaw = $dbRaw.Trim('"').Trim("'")
    if ($dbRaw) {
      if ([System.IO.Path]::IsPathRooted($dbRaw)) {
        $env:STEER_DB_PATH = $dbRaw
      } else {
        $env:STEER_DB_PATH = Join-Path $RepoPath $dbRaw
      }
    }
  }
}

& $binaryPath
exit $LASTEXITCODE
