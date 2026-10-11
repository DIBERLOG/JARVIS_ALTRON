param([switch]$CreateShortcut)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$build = Join-Path $repo 'target/release'
foreach ($name in @('jarvis-app.exe', 'jarvis-gui.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $build $name))) { throw "Не найден $name. Сначала выполните tools/build-windows.ps1." }
}
if ($CreateShortcut) {
    $desktop = [Environment]::GetFolderPath('Desktop')
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut((Join-Path $desktop 'JARVIS ALTRON.lnk'))
    $shortcut.TargetPath = "$env:SystemRoot/System32/WindowsPowerShell/v1.0/powershell.exe"
    $shortcut.Arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '"'
    $shortcut.WorkingDirectory = $repo
    $shortcut.IconLocation = "$build/jarvis-gui.exe,0"
    $shortcut.Save()
    Write-Host 'Ярлык JARVIS ALTRON создан на рабочем столе.'
    return
}
foreach ($name in @('jarvis-app', 'jarvis-gui')) {
    if (Get-Process -Name $name -ErrorAction SilentlyContinue) {
        Write-Host "$name уже запущен; второй экземпляр не запускается."
    } else {
        Start-Process -FilePath (Join-Path $build "$name.exe") -WorkingDirectory $build -WindowStyle Hidden
    }
}
