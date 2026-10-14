param([ValidateSet('emulator-5580','emulator-5582')][string]$Serial='emulator-5582')
$ErrorActionPreference='Stop'
$adb=Join-Path $env:LOCALAPPDATA 'Android/Sdk/platform-tools/adb.exe'
if((& $adb -s $Serial shell getprop ro.kernel.qemu).Trim() -ne '1'){throw 'Only the dedicated emulator is allowed.'}
$package='com.diberlog.jarvis.mobile.syncv2lab'
$component="$package/com.diberlog.jarvis.mobile.MainActivity"
& $adb -s $Serial logcat -c
for($attempt=1;$attempt -le 5;$attempt++) {
    & $adb -s $Serial shell am force-stop $package
    $launch=& $adb -s $Serial shell am start -W -n $component
    if($LASTEXITCODE -ne 0 -or ($launch -join "`n") -notmatch 'LaunchState: COLD'){throw "Cold launch failed: $attempt"}
    Start-Sleep -Milliseconds 1000
    $processId=& $adb -s $Serial shell pidof $package
    if(-not $processId){throw "App exited: $attempt"}
    $activities=& $adb -s $Serial shell dumpsys activity activities
    $foreground=$activities | Where-Object {$_ -match 'ResumedActivity' -and $_ -match [regex]::Escape($component)}
    if(-not $foreground){throw "MainActivity is not foreground: $attempt"}
    Write-Host "PASS cold launch ${attempt}: process and foreground screen"
}
$crashes=& $adb -s $Serial logcat -d -b crash
if(($crashes -join "`n") -match 'FATAL EXCEPTION'){throw 'Android crash buffer is not empty.'}
Write-Host 'PASS: crash buffer has no fatal exception.'
