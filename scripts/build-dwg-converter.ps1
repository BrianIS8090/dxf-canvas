param([string]$Dotnet = 'dotnet')

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_GENERATE_ASPNET_CERTIFICATE = 'false'
Push-Location -LiteralPath $projectRoot
try {
  & $Dotnet publish 'tools/dwg-converter/DwgConverter.csproj' -p:RestoreLockedMode=true -r win-x64 -c Release -p:OS=Windows_NT -o 'target/dwg-converter'
  if ($LASTEXITCODE -ne 0) { throw 'Не удалось собрать встроенный DWG-конвертер' }
  $converter = Join-Path $projectRoot 'target/dwg-converter/DxfCanvas.DwgConverter.exe'
  if (-not (Test-Path -LiteralPath $converter)) { throw 'Не получен EXE конвертера' }
  Get-Item -LiteralPath $converter | Select-Object Name, Length
} finally {
  Pop-Location
}
