$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '../resources/commands/vpn/vpn.ps1')
$script:events=[Collections.Generic.List[string]]::new()
function Get-VpnPath([string]$Name){return 'C:/fictional/'+$Name+'.exe'}
function Open-Vpn([string]$Path){$script:events.Add('open:'+([IO.Path]::GetFileName($Path)))}
function Invoke-Warp([string]$Path,[string]$Operation){$script:events.Add($Operation);if($script:rejectDisconnect -and $Operation -eq 'disconnect'){throw 'fixture rejection'}}
function Get-VpnProcesses([string]$Path){return @()}
Invoke-Vpn 'Happ' 'Open';Invoke-Vpn 'Happ' 'Restart';Invoke-Vpn 'Cloudflare' 'Restart'
if(($script:events -join ',') -ne 'open:Happ.exe,open:Happ.exe,disconnect,connect,open:Cloudflare.exe'){throw 'Неверный порядок запуска/переподключения'}
$script:events.Clear();$script:rejectDisconnect=$true
try{Invoke-Vpn 'Cloudflare' 'Restart';throw 'Отказ не обработан'}catch{if($_.Exception.Message -ne 'fixture rejection'){throw}}
if(($script:events -join ',') -ne 'disconnect'){throw 'После отказа выполняется нежелательная команда'}
$script:events.Clear();$script:waits=0
$fixture=[PSCustomObject]@{Id=123;StartTime=[DateTime]::UtcNow}
$fixture | Add-Member ScriptMethod CloseMainWindow {$script:events.Add('close');return $false}
$fixture | Add-Member ScriptMethod WaitForExit {param($timeout);$script:waits++;return $script:waits -gt 1}
function Get-VpnProcesses([string]$Path){return @($fixture)}
function Stop-Process {param($InputObject,[switch]$Force,$ErrorAction);if($InputObject -ne $fixture -or -not $Force){throw 'Чужой процесс'};$script:events.Add('stop-fixture')}
Restart-Happ 'C:/fictional/Happ.exe'
if(($script:events -join ',') -ne 'close,stop-fixture,open:Happ.exe'){throw 'Неверный перезапуск клиента в трее'}
Write-Host 'PASS: запуск, перезапуск остановленного/работающего Happ, переподключение и отказ; настоящие VPN не изменены.'
