param([string]$OutputDirectory = 'dist/release')

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
  $metadataJson = cargo metadata --locked --no-deps --format-version 1
  if ($LASTEXITCODE -ne 0) { throw 'Не удалось прочитать версию проекта' }
  $package = ($metadataJson | ConvertFrom-Json).packages | Where-Object name -EQ 'dxf-canvas'
  $version = $package.version
  if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Ожидается версия X.Y.Z' }
  if ($env:GITHUB_REF_TYPE -eq 'tag' -and $env:GITHUB_REF_NAME -ne "v$version") {
    throw 'Тег релиза не совпадает с версией Cargo.toml'
  }
  $source = Join-Path $projectRoot 'target/release/dxf-canvas.exe'
  if (-not (Test-Path -LiteralPath $source)) { throw 'Сначала выполните cargo build --locked --release' }
  $destination = Join-Path $projectRoot $OutputDirectory
  New-Item -ItemType Directory -Path $destination -Force | Out-Null
  $name = "DXF-Canvas-$version-windows-x64.exe"
  $asset = Join-Path $destination $name
  Copy-Item -LiteralPath $source -Destination $asset -Force
  & "$PSScriptRoot/sign.ps1" -Path $asset
  $licenseFiles = @('LICENSE', 'THIRD_PARTY_NOTICES.md', 'docs/licenses/OPEN-SANS-OFL.txt', 'docs/licenses/ACADSHARP-LICENSE.txt', 'docs/licenses/DOTNET-LICENSE.txt', 'docs/licenses/DOTNET-NATIVE-NOTICES.txt')
  $notices = ($licenseFiles | ForEach-Object { Get-Content -LiteralPath $_ -Raw }) -join "`n`n"
  [System.IO.File]::WriteAllText((Join-Path $destination 'THIRD-PARTY-LICENSES.txt'), $notices, [System.Text.UTF8Encoding]::new($false))
  $payload = Join-Path $projectRoot 'dist/payload'
  New-Item -ItemType Directory -Path $payload -Force | Out-Null
  Copy-Item -LiteralPath $asset -Destination "$payload/dxf-canvas.exe" -Force
  Copy-Item -LiteralPath "$destination/THIRD-PARTY-LICENSES.txt" -Destination $payload -Force
  Copy-Item -LiteralPath "$projectRoot/LICENSE" -Destination "$payload/LICENSE.txt" -Force
  & "$PSScriptRoot/build-installer.ps1" -PayloadDirectory $payload -OutputDirectory $destination
  $signature = Get-AuthenticodeSignature -LiteralPath $asset
  $signing = [ordered]@{ version = $version; authenticode = $signature.Status.ToString(); publisher = if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { $null } }
  $signing | ConvertTo-Json | Set-Content -LiteralPath "$destination/SIGNING.json" -Encoding UTF8
  $files = @($name, "DXF-Canvas-$version-setup-x64.exe", 'THIRD-PARTY-LICENSES.txt', 'SIGNING.json')
  $sums = foreach ($file in $files) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $destination $file) -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $file"
  }
  [System.IO.File]::WriteAllText((Join-Path $destination 'SHA256SUMS.txt'), ($sums -join "`n") + "`n", [System.Text.UTF8Encoding]::new($false))
  Write-Output "Подготовлен выпуск $version"
} finally {
  Pop-Location
}
