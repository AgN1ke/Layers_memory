param(
    [switch]$NoBuild,
    [switch]$NoStart,
    [switch]$NoStop,
    [switch]$Visible,
    [switch]$NoDevSleepNotices,
    [switch]$ClearMemory,
    [switch]$TailLog
)

$ErrorActionPreference = "Stop"

chcp 65001 | Out-Null
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new()
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$OutputEncoding = [System.Text.UTF8Encoding]::new()
$env:PYTHONIOENCODING = "utf-8"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Resolve-Path (Join-Path $ScriptDir "..\..")
$Venv = Join-Path $Root "crates\python_adapter\.venv"
$Python = Join-Path $Venv "Scripts\python.exe"
$Maturin = Join-Path $Venv "Scripts\maturin.exe"
$RuntimeDir = Join-Path $ScriptDir "runtime"
$MemoryDir = Join-Path $RuntimeDir "memory"
$StateDir = Join-Path $RuntimeDir "state"
$SecretsPath = Join-Path $StateDir "secrets.local.json"
$LogPath = Join-Path $RuntimeDir "logs\bot.log"
$BotPath = Join-Path $ScriptDir "bot.py"

function Get-BotProcesses {
    $rootText = [string]$Root
    Get-CimInstance Win32_Process |
        Where-Object {
            ($_.Name -eq "python.exe" -or $_.Name -eq "pythonw.exe") -and
            $_.CommandLine -and
            $_.CommandLine.Contains($rootText) -and
            $_.CommandLine.Contains("telegram_gemini_bot") -and
            $_.CommandLine.Contains("bot.py")
        }
}

function Stop-BotProcesses {
    $processes = @(Get-BotProcesses)
    if ($processes.Count -eq 0) {
        Write-Host "No running Telegram bot process found."
        return
    }

    Write-Host "Stopping $($processes.Count) Telegram bot process(es)..."
    foreach ($process in $processes) {
        Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Seconds 1
}

function Assert-ChildPath {
    param(
        [Parameter(Mandatory = $true)][string]$Parent,
        [Parameter(Mandatory = $true)][string]$Child
    )

    $parentFull = [System.IO.Path]::GetFullPath($Parent).TrimEnd('\') + '\'
    $childFull = [System.IO.Path]::GetFullPath($Child).TrimEnd('\')
    if (-not $childFull.StartsWith($parentFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to modify path outside expected parent. parent=$parentFull child=$childFull"
    }
}

function Clear-MemoryRuntime {
    if (-not (Test-Path $MemoryDir)) {
        Write-Host "Memory runtime does not exist; nothing to clear."
        return
    }

    Assert-ChildPath -Parent $RuntimeDir -Child $MemoryDir
    Write-Host "Clearing memory runtime: $MemoryDir"
    Remove-Item -LiteralPath $MemoryDir -Recurse -Force
    New-Item -ItemType Directory -Force -Path $MemoryDir | Out-Null
}

function Ensure-Venv {
    if (-not (Test-Path $Python)) {
        Write-Host "Creating Python venv for Memory Engine adapter..."
        py -3.13 -m venv $Venv
    }

    if (-not (Test-Path $Maturin)) {
        Write-Host "Installing maturin and pytest into adapter venv..."
        & $Python -m pip install --upgrade pip maturin pytest | Out-Host
    }
}

function Read-SecretCache {
    if (-not (Test-Path $SecretsPath)) {
        return $null
    }
    return Get-Content -Path $SecretsPath -Raw -Encoding UTF8 | ConvertFrom-Json
}

function Set-EnvFromSecretCache {
    $secrets = Read-SecretCache
    if (-not $env:TELEGRAM_BOT_TOKEN -and $secrets -and $secrets.telegram_token) {
        $env:TELEGRAM_BOT_TOKEN = [string]$secrets.telegram_token
    }
    if (-not $env:GEMINI_API_KEY -and $secrets -and $secrets.gemini_api_key) {
        $env:GEMINI_API_KEY = [string]$secrets.gemini_api_key
    }

    if ($secrets) {
        if ($secrets.reasoning_model) { $env:GEMINI_REASONING_MODEL = [string]$secrets.reasoning_model }
        if ($secrets.balanced_model) { $env:GEMINI_BALANCED_MODEL = [string]$secrets.balanced_model }
        if ($secrets.fast_model) { $env:GEMINI_FAST_MODEL = [string]$secrets.fast_model }
        if ($secrets.chat_role) { $env:MEMORY_BOT_CHAT_ROLE = [string]$secrets.chat_role }
    }

    if (-not $env:TELEGRAM_BOT_TOKEN) {
        throw "TELEGRAM_BOT_TOKEN is missing. Use run_gui.ps1 once to save runtime/state/secrets.local.json, or set env."
    }
    if (-not $env:GEMINI_API_KEY) {
        throw "GEMINI_API_KEY is missing. Use run_gui.ps1 once to save runtime/state/secrets.local.json, or set env."
    }

    $env:MEMORY_BOT_NONINTERACTIVE = "1"
    if ($NoDevSleepNotices) {
        Remove-Item Env:MEMORY_BOT_DEV_SLEEP_NOTICES -ErrorAction SilentlyContinue
    } else {
        $env:MEMORY_BOT_DEV_SLEEP_NOTICES = "1"
    }
}

function Build-Adapter {
    if ($NoBuild) {
        Write-Host "Skipping maturin build because -NoBuild was passed."
        return
    }

    Push-Location (Join-Path $Root "crates\python_adapter")
    try {
        Write-Host "Running maturin develop..."
        & $Maturin develop | Out-Host
    }
    finally {
        Pop-Location
    }
}

function Start-Bot {
    if ($NoStart) {
        Write-Host "Skipping bot start because -NoStart was passed."
        return
    }

    $windowStyle = if ($Visible) { "Normal" } else { "Hidden" }
    Write-Host "Starting Telegram bot (dev sleep notices: $(-not $NoDevSleepNotices))..."
    Start-Process -WindowStyle $windowStyle -FilePath $Python -ArgumentList "`"$BotPath`"" -WorkingDirectory $Root
    Start-Sleep -Seconds 3

    $processes = @(Get-BotProcesses)
    Write-Host "Running Telegram bot process count: $($processes.Count)"
    foreach ($process in $processes) {
        Write-Host "  pid=$($process.ProcessId) parent=$($process.ParentProcessId)"
    }

    if (Test-Path $LogPath) {
        Write-Host ""
        Write-Host "Last bot log lines:"
        Get-Content -Path $LogPath -Tail 8 -Encoding UTF8
    }
}

if (-not $NoStop) {
    Stop-BotProcesses
}

if ($ClearMemory) {
    Clear-MemoryRuntime
}

Ensure-Venv
Set-EnvFromSecretCache
Build-Adapter
Start-Bot

if ($TailLog) {
    Write-Host ""
    Write-Host "Tailing bot log. Press Ctrl+C to stop tailing."
    Get-Content -Path $LogPath -Wait -Tail 40 -Encoding UTF8
}
