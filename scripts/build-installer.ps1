param(
  [string]$PayloadDirectory = 'dist/payload',
  [string]$OutputDirectory = 'dist/release',
  [string]$Compiler = $env:DXF_ISCC,
  [switch]$TestIdentity
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$payload = if ([IO.Path]::IsPathRooted($PayloadDirectory)) { [IO.Path]::GetFullPath($PayloadDirectory) } else { [IO.Path]::GetFullPath((Join-Path $projectRoot $PayloadDirectory)) }
$output = if ([IO.Path]::IsPathRooted($OutputDirectory)) { [IO.Path]::GetFullPath($OutputDirectory) } else { [IO.Path]::GetFullPath((Join-Path $projectRoot $OutputDirectory)) }
if (!$Compiler) { $Compiler = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
if (!(Test-Path -LiteralPath $Compiler)) { throw 'Установите Inno Setup 6 или задайте DXF_ISCC' }
foreach ($name in @('dxf-canvas.exe', 'LICENSE.txt', 'THIRD-PARTY-LICENSES.txt')) {
  if (!(Test-Path -LiteralPath (Join-Path $payload $name))) { throw "Отсутствует $name" }
}
$version = (Get-Item -LiteralPath "$payload\dxf-canvas.exe").VersionInfo.ProductVersion
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'EXE не содержит корректную версию' }
$cargoVersion = (Select-String -LiteralPath "$projectRoot\Cargo.toml" -Pattern '^version = "([^"]+)"').Matches.Groups[1].Value
if ($version -ne $cargoVersion) { throw 'Версия EXE отличается от Cargo.toml' }
New-Item -ItemType Directory -Force -Path $output | Out-Null
$arguments = @("/DPayloadDir=$payload", "/DOutputDir=$output", "/DAppVersion=$version")
if ($TestIdentity) {
  $arguments += @('/DAppIdValue=DXFCanvasInstallerTest', '/DAppName=DXF Canvas Installer Test', '/DRegistryName=DXFCanvasInstallerTest', '/DInstallFolder=DXF Canvas Installer Test')
}
if ($env:DXF_SIGN_CERT_THUMBPRINT) {
  if ((Get-AuthenticodeSignature -LiteralPath "$payload\dxf-canvas.exe").Status -ne 'Valid') { throw 'EXE должен быть подписан до упаковки' }
  $signScript = Join-Path $PSScriptRoot 'sign.ps1'
  $arguments += '/DSignedBuild=1'
  $arguments += "/Sdxf=powershell.exe -NoProfile -ExecutionPolicy Bypass -File `$q$signScript`$q -Path `$f"
} elseif ($env:DXF_REQUIRE_SIGNING -eq '1') {
  throw 'Для этого выпуска обязательна доверенная подпись'
}
& $Compiler @arguments "$projectRoot\installer\DXF-Canvas.iss"
if ($LASTEXITCODE -ne 0) { throw 'Ошибка сборки установщика' }
$installer = Join-Path $output "DXF-Canvas-$version-setup-x64.exe"
if (!(Test-Path -LiteralPath $installer)) { throw 'Установщик не создан' }
if ($env:DXF_SIGN_CERT_THUMBPRINT -and (Get-AuthenticodeSignature -LiteralPath $installer).Status -ne 'Valid') {
  throw 'Подпись установщика не прошла проверку'
}
Write-Output "Установщик: $installer"
