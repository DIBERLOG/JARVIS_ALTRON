$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repo
$compiler = $env:JARVIS_AHK_COMPILER
if (-not $compiler) { $compiler = Join-Path ${env:ProgramFiles} 'AutoHotkey/Compiler/Ahk2Exe.exe' }
$scripts = @(Get-ChildItem -LiteralPath "$repo/resources/commands" -Filter '*.ahk' -Recurse | Where-Object { $_.Name -ne '_include.ahk' })
$scripts += Get-Item -LiteralPath "$repo/resources/tts/Speak.ahk"
foreach ($script in $scripts) {
    $output = [IO.Path]::ChangeExtension($script.FullName, '.exe')
    if (-not (Test-Path -LiteralPath $output)) {
        if (-not (Test-Path -LiteralPath $compiler)) { throw 'Для компиляции AHK установите AutoHotkey v1 с Ahk2Exe либо задайте JARVIS_AHK_COMPILER.' }
        $process = Start-Process -FilePath $compiler -ArgumentList @('/in', ('"' + $script.FullName + '"'), '/out', ('"' + $output + '"'), '/silent') -WindowStyle Hidden -PassThru -Wait
        if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $output)) { throw "Не удалось скомпилировать $($script.Name)." }
    }
}
& "$PSScriptRoot/build-windows.ps1"
if (-not (Test-Path -LiteralPath "$repo/target/release/jarvis-app.exe")) { throw 'Голосовое ядро не собрано.' }
$previousLib = $env:LIB
try {
    $env:LIB = "$repo/lib/windows/amd64;$previousLib"
    Push-Location -LiteralPath "$repo/crates/jarvis-gui"
    try {
        & "$repo/frontend/node_modules/.bin/tauri.cmd" build --bundles nsis
        if ($LASTEXITCODE -ne 0) { throw 'Не удалось собрать Windows-установщик.' }
    } finally { Pop-Location }
} finally { $env:LIB = $previousLib }
Write-Host 'Установщик: target/release/bundle/nsis. Проверяйте на чистой Windows перед публикацией.'
