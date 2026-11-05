# Starts the assistant only after Voicemod has had time to initialize its audio devices.
# This script is invoked by the per-user "Jarvis + Voicemod" Startup shortcut.

$ErrorActionPreference = 'Stop'

$voicemodCandidates = @(
    'C:\Program Files\Voicemod V3\Voicemod.exe',
    (Join-Path $env:LOCALAPPDATA 'VoicemodV3\app\last\Voicemod.exe')
)
$voicemod = $voicemodCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
$jarvisRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$jarvisBin = Join-Path $jarvisRoot 'target\release'
$jarvisApp = Join-Path $jarvisBin 'jarvis-app-sync-v2-lab.exe'
$jarvisGui = Join-Path $jarvisBin 'jarvis-gui-sync-v2-lab.exe'
$jarvisGuiProcessName = [System.IO.Path]::GetFileNameWithoutExtension($jarvisGui)
$logFile = Join-Path $env:APPDATA 'com.priler.jarvis\startup.log'

function Start-IfNotRunning {
    param(
        [Parameter(Mandatory = $true)][string]$ProcessName,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string]$WorkingDirectory
    )

    $alreadyRunning = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -eq $FilePath
    }
    if (-not $alreadyRunning) {
        $startParams = @{ FilePath = $FilePath }
        if ($WorkingDirectory) {
            $startParams.WorkingDirectory = $WorkingDirectory
        }
        if ($FilePath -ne $jarvisGui) { $startParams.WindowStyle = 'Hidden' }
        Start-Process @startParams
    }
}

try {
    if (-not (Test-Path -LiteralPath $jarvisApp)) { throw "Jarvis engine not found: $jarvisApp" }
    if (-not (Test-Path -LiteralPath $jarvisGui)) { throw "Jarvis interface not found: $jarvisGui" }

    if ($voicemod -and -not (Get-Process -Name 'Voicemod' -ErrorAction SilentlyContinue)) {
        Start-Process -FilePath $voicemod -WindowStyle Hidden
    }
    # Login-time audio and WebView2 initialization can outlive the Startup shortcut.
    Start-Sleep -Seconds 20
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        # The GUI owns and starts the voice process; do not create a second core here.
        Start-IfNotRunning -ProcessName $jarvisGuiProcessName -FilePath $jarvisGui -WorkingDirectory $jarvisBin
        Start-Sleep -Seconds 5
        $gui = Get-Process -Name $jarvisGuiProcessName -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $jarvisGui }
        if ($gui) { break }
        "$(Get-Date -Format o) GUI start attempt $attempt failed" | Add-Content -LiteralPath $logFile
    }
    if (-not $gui) { throw 'Jarvis GUI did not remain running after three attempts' }
    "$(Get-Date -Format o) Started Jarvis; Voicemod present: $([bool]$voicemod)" | Add-Content -LiteralPath $logFile
} catch {
    "$(Get-Date -Format o) ERROR: $_" | Add-Content -LiteralPath $logFile
    throw
}
