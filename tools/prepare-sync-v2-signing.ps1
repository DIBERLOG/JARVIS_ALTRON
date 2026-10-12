$ErrorActionPreference = 'Stop'
if (-not $env:JAVA_HOME) { throw 'Укажите JAVA_HOME для JDK 17.' }
$signingDirectory = Join-Path $env:USERPROFILE '.codex\jarvis-sync-v2-lab-signing'
New-Item -ItemType Directory -Force -Path $signingDirectory | Out-Null
$store = Join-Path $signingDirectory 'jarvis-sync-v2-lab.p12'
$protectedPassword = Join-Path $signingDirectory 'password.dpapi'
if ((Test-Path -LiteralPath $store) -ne (Test-Path -LiteralPath $protectedPassword)) { throw 'Неполный ключ подписи. Не создавайте замену; восстановите исходные файлы.' }
if (-not (Test-Path -LiteralPath $store)) {
    $passwordBytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Fill($passwordBytes)
    $env:JARVIS_LAB_SIGNING_PASSWORD = [Convert]::ToBase64String($passwordBytes)
    [Array]::Clear($passwordBytes, 0, $passwordBytes.Length)
    $securePassword = ConvertTo-SecureString $env:JARVIS_LAB_SIGNING_PASSWORD -AsPlainText -Force
    ConvertFrom-SecureString $securePassword | Set-Content -LiteralPath $protectedPassword
    & (Join-Path $env:JAVA_HOME 'bin\keytool.exe') -genkeypair -keystore $store -storetype PKCS12 -storepass:env JARVIS_LAB_SIGNING_PASSWORD -keypass:env JARVIS_LAB_SIGNING_PASSWORD -alias jarvis-sync-v2-lab -keyalg RSA -keysize 3072 -validity 10000 -dname 'CN=JARVIS Sync v2 LAB, O=DIBERLOG'
    if ($LASTEXITCODE -ne 0) { throw 'Не удалось создать постоянный LAB-ключ; сохранённый пароль оставлен для восстановления.' }
} else {
    $securePassword = (Get-Content -Raw -LiteralPath $protectedPassword).Trim() | ConvertTo-SecureString
    $env:JARVIS_LAB_SIGNING_PASSWORD = [Net.NetworkCredential]::new('', $securePassword).Password
}
$env:JARVIS_LAB_KEYSTORE = $store
Write-Host 'Постоянный ключ Sync v2 LAB готов. Секреты подписи не входят в репозиторий и APK.'
