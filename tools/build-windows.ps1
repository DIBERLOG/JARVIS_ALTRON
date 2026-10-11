param([switch]$CheckOnly)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repo
foreach ($program in @('cargo', 'npm.cmd')) {
    if (-not (Get-Command $program -ErrorAction SilentlyContinue)) { throw "Не найден $program. См. docs/INSTALL.md." }
}
if (-not (Test-Path -LiteralPath "$repo/lib/windows/amd64/libvosk.lib")) { throw 'Нет библиотеки Vosk: lib/windows/amd64/libvosk.lib.' }
if ($CheckOnly) { Write-Host 'Cargo, npm и библиотека Vosk найдены. C++ Build Tools проверяются при сборке.'; return }
$redist = $env:JARVIS_VC_REDIST_DIR
if (-not $redist) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (Test-Path -LiteralPath $vswhere) {
        $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($installation) {
            $redist = Get-ChildItem -LiteralPath "$installation/VC/Redist/MSVC" -Directory | Sort-Object Name -Descending |
                ForEach-Object { Join-Path $_.FullName 'x64/Microsoft.VC143.CRT' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        }
    }
}
if (-not $redist) { throw 'Не найден x64 Visual C++ Redist. Установите C++ Build Tools либо задайте JARVIS_VC_REDIST_DIR.' }
New-Item -ItemType Directory -Path "$repo/target/release" -Force | Out-Null
foreach ($dll in @('msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')) {
    Copy-Item -LiteralPath (Join-Path $redist $dll) -Destination "$repo/target/release"
}
Push-Location -LiteralPath "$repo/frontend"
try {
    & npm.cmd ci
    if ($LASTEXITCODE -ne 0) { throw 'Не удалось установить зависимости интерфейса.' }
    & npm.cmd run build
    if ($LASTEXITCODE -ne 0) { throw 'Ошибка сборки интерфейса.' }
} finally { Pop-Location }
$previousLib = $env:LIB
try {
    $env:LIB = "$repo/lib/windows/amd64;$previousLib"
    & cargo build --locked --release -p jarvis-app -p jarvis-gui
    if ($LASTEXITCODE -ne 0) { throw 'Ошибка сборки приложения. Проверьте C++ Build Tools и Windows SDK.' }
} finally { $env:LIB = $previousLib }
$destination = "$repo/target/release"
if (Test-Path -LiteralPath "$destination/resources") {
    Write-Host 'Существующие ресурсы сохранены. Перед их ручным обновлением сделайте резервную копию.'
} else { Copy-Item -LiteralPath "$repo/resources" -Destination $destination -Recurse }
Get-ChildItem -LiteralPath "$repo/lib/windows/amd64" -Filter '*.dll' | Copy-Item -Destination $destination
Write-Host 'Готово. Запуск: powershell -File tools/start-windows.ps1'
