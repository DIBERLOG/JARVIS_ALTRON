param([Parameter(Mandatory=$true)][string]$Directory)
$ErrorActionPreference = 'Stop'
$package = (Resolve-Path -LiteralPath $Directory).Path
foreach ($relative in @('jarvis-gui.exe','jarvis-app.exe','libvosk.dll','DirectML.dll','msvcp140.dll','msvcp140_1.dll','vcruntime140.dll','vcruntime140_1.dll','resources/tts/Speak.exe','resources/commands','resources/vosk/vosk-model-small-ru-0.22/am/final.mdl','LICENSE.txt','THIRD_PARTY_NOTICES.md')) {
    if (-not (Test-Path -LiteralPath (Join-Path $package $relative))) { throw "В пакете отсутствует $relative" }
}
$private = Get-ChildItem -LiteralPath $package -File -Recurse | Where-Object {
    $_.Name -in @('app.db','password-vault-v1.json','.env','trained-checkpoint.txt') -or $_.Extension -in @('.pth','.ckpt')
}
if ($private) { throw 'В пакет попали локальные настройки или обученные модели.' }
foreach ($binary in @('jarvis-app.exe','jarvis-gui.exe')) {
    $process = Start-Process -FilePath (Join-Path $package $binary) -ArgumentList '--release-smoke-test' -WorkingDirectory $package -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "${binary}: проверка запуска завершилась кодом $($process.ExitCode)" }
}
Write-Host 'Оба процесса запускаются, обязательные ресурсы присутствуют, приватные файлы не обнаружены.'
Write-Host 'Это не проверка микрофона, почты или установки на чистом компьютере.'
