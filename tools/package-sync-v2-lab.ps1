$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
$destination = Join-Path $repository 'artifacts\sync-v2-lab'
$windows = Join-Path $destination 'windows'
New-Item -ItemType Directory -Force -Path $windows | Out-Null
Copy-Item -LiteralPath "$repository\target\release\jarvis-gui-sync-v2-lab.exe" -Destination "$windows\JARVIS-Sync-v2-LAB.exe"
Copy-Item -LiteralPath "$repository\target\release\jarvis-app-sync-v2-lab.exe" -Destination $windows
Get-ChildItem -LiteralPath "$repository\lib\windows\amd64" -Filter '*.dll' | Copy-Item -Destination $windows
foreach ($name in @('DirectML.dll','msvcp140.dll','msvcp140_1.dll','vcruntime140.dll','vcruntime140_1.dll')) {
    $source = Join-Path "$repository\target\release" $name
    if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination $windows }
}
$resources = Join-Path $windows 'resources'
New-Item -ItemType Directory -Force -Path $resources | Out-Null
foreach ($name in @('commands','sound','vosk','rustpotter','keywords')) {
    Copy-Item -LiteralPath (Join-Path "$repository\resources" $name) -Destination $resources -Recurse -Force
}
New-Item -ItemType Directory -Force -Path "$resources\tts","$windows\third_party" | Out-Null
foreach ($name in @('AudioOutput.py','SileroSpeak.py','XttsSpeak.py','S2Speak.py','NeuralSpeak.py','Speak.ahk','Speak.exe','MicMonitor.py')) { Copy-Item -LiteralPath (Join-Path "$repository\resources\tts" $name) -Destination "$resources\tts" }
Copy-Item -LiteralPath "$repository\LICENSE.txt","$repository\THIRD_PARTY_NOTICES.md" -Destination $windows
Copy-Item -LiteralPath "$repository\third_party\licenses" -Destination "$windows\third_party" -Recurse -Force
Copy-Item -LiteralPath "$repository\docs\SYNC_V2_LAB_TESTING.md" -Destination "$windows\READ-ME-SYNC-v2-LAB.md"
Copy-Item -LiteralPath "$repository\mobile\android\app\build\outputs\apk\release\app-release.apk" -Destination "$destination\JARVIS-Mobile-1.1.0-Sync-v2-LAB.apk"
& "$windows\JARVIS-Sync-v2-LAB.exe" --release-smoke-test
if ($LASTEXITCODE -ne 0) { throw 'Проверка комплекта Windows не прошла.' }
$compiler = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'
if (-not (Test-Path -LiteralPath $compiler)) { throw 'Установите NSIS или подготовьте его через Tauri.' }
Push-Location -LiteralPath $PSScriptRoot
try { & $compiler 'sync-v2-lab-installer.nsi'; if ($LASTEXITCODE -ne 0) { throw 'Ошибка LAB-установщика.' } } finally { Pop-Location }
Get-ChildItem -LiteralPath $destination -File | Where-Object Extension -In '.exe','.apk' | ForEach-Object { $hash=Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256; "$($hash.Hash.ToLowerInvariant())  $($_.Name)" } | Set-Content -LiteralPath "$destination\SHA256SUMS.txt"
Write-Host "Тестовые сборки: $destination"
