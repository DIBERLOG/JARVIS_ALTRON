$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$addresses = @(Get-NetIPConfiguration -All | ForEach-Object {
    $configuration = $_
    foreach ($address in $configuration.IPv4Address) {
        [pscustomobject]@{
            address = $address.IPAddress
            physical = [bool]$configuration.NetAdapter.HardwareInterface
            up = $configuration.NetAdapter.Status -eq 'Up'
            gateway = [bool]$configuration.IPv4DefaultGateway
            metric = [int]$configuration.NetIPv4Interface.InterfaceMetric
        }
    }
})
ConvertTo-Json -InputObject $addresses -Compress
