$ErrorActionPreference = "Stop"

chcp 65001 | Out-Null
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new()
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Resolve-Path (Join-Path $ScriptDir "..\..")
$Venv = Join-Path $Root "crates\python_adapter\.venv"
$Python = Join-Path $Venv "Scripts\python.exe"
$Maturin = Join-Path $Venv "Scripts\maturin.exe"

if (-not (Test-Path $Python)) {
    Write-Host "Creating Python venv for Memory Engine adapter..."
    py -3.13 -m venv $Venv
}

& $Python -m pip install --upgrade pip | Out-Host
& $Python -m pip install maturin pytest | Out-Host

Push-Location (Join-Path $Root "crates\python_adapter")
try {
    & $Maturin develop | Out-Host
}
finally {
    Pop-Location
}

& $Python (Join-Path $ScriptDir "local_harness.py") @args
