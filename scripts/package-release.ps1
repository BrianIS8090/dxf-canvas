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
  $destination = Join-Path $projectRoot 'dist/release'
  New-Item -ItemType Directory -Path $destination -Force | Out-Null
  $name = "DXF-Canvas-$version-windows-x64.exe"
  $asset = Join-Path $destination $name
  Copy-Item -LiteralPath $source -Destination $asset -Force
  $hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
  [System.IO.File]::WriteAllText((Join-Path $destination 'SHA256SUMS.txt'), "$hash  $name`n", [System.Text.UTF8Encoding]::new($false))
  Write-Output "Подготовлен $name; SHA-256: $hash"
} finally {
  Pop-Location
}
