param([Parameter(Mandatory=$true)][string]$Path)
$ErrorActionPreference = 'Stop'
$file = (Resolve-Path -LiteralPath $Path).Path
$thumbprint = $env:DXF_SIGN_CERT_THUMBPRINT
if (!$thumbprint) {
  if ($env:DXF_REQUIRE_SIGNING -eq '1') { throw 'Для выпуска требуется DXF_SIGN_CERT_THUMBPRINT' }
  Write-Output "Без цифровой подписи: $([IO.Path]::GetFileName($file))"
  return
}
if ($thumbprint -notmatch '^[0-9A-Fa-f]{40}$') { throw 'Некорректный отпечаток сертификата' }
$store = if ($env:DXF_SIGN_MACHINE_STORE -eq '1') { 'LocalMachine' } else { 'CurrentUser' }
$certificate = Get-Item -LiteralPath "Cert:\$store\My\$thumbprint"
if (!$certificate.HasPrivateKey -or $certificate.NotAfter -le (Get-Date) -or $certificate.NotBefore -gt (Get-Date)) {
  throw 'Сертификат не действителен или не содержит закрытого ключа'
}
if (!($certificate.EnhancedKeyUsageList | Where-Object { $_.ObjectId.Value -eq '1.3.6.1.5.5.7.3.3' })) {
  throw 'Сертификат не предназначен для подписи программ'
}
$signTool = $env:DXF_SIGNTOOL
if (!$signTool) {
  $signTool = Get-ChildItem -Path "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
    Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (!$signTool -or !(Test-Path -LiteralPath $signTool)) { throw 'Не найден SignTool из Windows SDK' }
$arguments = @('sign', '/sha1', $thumbprint, '/s', 'My', '/fd', 'SHA256', '/tr', 'http://timestamp.digicert.com', '/td', 'SHA256')
if ($store -eq 'LocalMachine') { $arguments += '/sm' }
& $signTool @arguments $file
if ($LASTEXITCODE -ne 0) { throw 'Ошибка подписи файла' }
$signature = Get-AuthenticodeSignature -LiteralPath $file
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $thumbprint -or !$signature.TimeStamperCertificate) {
  throw 'Подпись, цепочка доверия, издатель или метка времени не прошли проверку'
}
& $signTool verify /pa /all $file
if ($LASTEXITCODE -ne 0) { throw 'SignTool не подтвердил подпись' }
