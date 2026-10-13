param(
    [ValidateSet('Happ','Cloudflare')][string]$Client='Happ',
    [ValidateSet('Open','Restart','Inspect')][string]$Action='Inspect'
)
$ErrorActionPreference='Stop'

function Get-VpnPath([string]$Name) {
    $relative=if($Name -eq 'Happ'){'FlyFrogLLC/Happ/Happ.exe'}else{'Cloudflare/Cloudflare WARP/Cloudflare WARP.exe'}
    $path=Join-Path $env:ProgramFiles $relative
    if(-not (Test-Path -LiteralPath $path -PathType Leaf)){throw "$Name не найден. Установите приложение в стандартную папку."}
    return [IO.Path]::GetFullPath($path)
}
function Get-VpnProcesses([string]$Path) {
    $session=(Get-Process -Id $PID).SessionId
    return @(Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($Path)) -ErrorAction SilentlyContinue | Where-Object {
        $_.SessionId -eq $session -and $_.Path -and [string]::Equals($_.Path,$Path,[StringComparison]::OrdinalIgnoreCase)
    })
}
function Open-Vpn([string]$Path) {
    # A second start brings a single-instance VPN client to the foreground. No shell string.
    Start-Process -FilePath $Path -WorkingDirectory ([IO.Path]::GetDirectoryName($Path)) -WindowStyle Normal | Out-Null
}
function Restart-Happ([string]$Path) {
    foreach($process in (Get-VpnProcesses $Path)) {
        $started=$process.StartTime.ToUniversalTime().Ticks
        [void]$process.CloseMainWindow()
        if(-not $process.WaitForExit(8000)) {
            # Recheck the exact executable and start time before stopping a tray-only client.
            $current=Get-VpnProcesses $Path | Where-Object {$_.Id -eq $process.Id -and $_.StartTime.ToUniversalTime().Ticks -eq $started}
            foreach($same in $current){Stop-Process -InputObject $same -Force -ErrorAction Stop;if(-not $same.WaitForExit(5000)){throw 'Happ не завершился. Повторите позже.'}}
        }
    }
    Open-Vpn $Path
}
function Invoke-Warp([string]$Path,[string]$Operation) {
    $cli=Join-Path ([IO.Path]::GetDirectoryName($Path)) 'warp-cli.exe'
    if(-not (Test-Path -LiteralPath $cli -PathType Leaf)){throw 'Штатная утилита Cloudflare не найдена.'}
    $process=Start-Process -FilePath $cli -ArgumentList @($Operation) -WindowStyle Hidden -PassThru
    if(-not $process.WaitForExit(15000)){throw 'Cloudflare не ответил вовремя. Проверьте окно клиента.'}
    if($process.ExitCode -ne 0){throw 'Cloudflare отклонил команду. Проверьте подключение и ограничения в окне клиента.'}
}
function Invoke-Vpn([string]$Name,[string]$Operation) {
    $path=Get-VpnPath $Name
    if($Operation -eq 'Inspect'){return [PSCustomObject]@{Client=$Name;Path=$path;Action=$Operation}}
    if($Operation -eq 'Open'){Open-Vpn $path;return}
    if($Name -eq 'Happ'){Restart-Happ $path;return}
    Invoke-Warp $path 'disconnect'
    Invoke-Warp $path 'connect'
    Open-Vpn $path
}
if($MyInvocation.InvocationName -eq '.'){return}
try {Invoke-Vpn $Client $Action} catch {
    Add-Type -AssemblyName System.Windows.Forms
    [void][System.Windows.Forms.MessageBox]::Show($_.Exception.Message,'JARVIS · VPN')
    exit 1
}
