param(
    [string]$Python = "python3"
)

$ErrorActionPreference = "Stop"

$Root = (Resolve-Path "$PSScriptRoot\..\..")
Set-Location $Root

$venvPath = "$Root\.venv"
if (-not (Test-Path $venvPath)) {
    & $Python -m venv $venvPath
}

$activate = Join-Path $venvPath "Scripts\Activate.ps1"
. $activate

pip install --upgrade pip
pip install -r requirements.txt

if (Get-Command pytest -ErrorAction SilentlyContinue) {
    try {
        pytest -q | Out-Null
    } catch {
        Write-Warning "Pytest run failed; inspect output above."
    }
} else {
    Write-Warning "pytest not installed; skipping test run"
}
