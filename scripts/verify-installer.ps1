param([string]$Compiler = $env:DXF_ISCC)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (!('DxfShortcut' -as [type])) { Add-Type -Path "$PSScriptRoot/ShortcutPath.cs" }
$root = [IO.Path]::GetFullPath((Join-Path $projectRoot 'test-output/installer'))
New-Item -ItemType Directory -Path $root -Force | Out-Null
$checks = [Collections.Generic.List[string]]::new()
function Assert-Check([bool]$Condition, [string]$Message) {
  if (!$Condition) { throw $Message }
  $checks.Add($Message)
}
function UserChoices {
  $values = foreach ($extension in @('.dxf', '.dwg')) {
    $key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$extension\UserChoice"
    if (Test-Path $key) { Get-ItemProperty $key | Select-Object ProgId, Hash } else { $null }
  }
  ConvertTo-Json -InputObject @($values) -Compress
}
$beforeChoices = UserChoices
$install = Join-Path $root 'Установка с пробелами'
$exe = Join-Path $install 'dxf-canvas.exe'
$uninstaller = Join-Path $install 'unins000.exe'
$registry = 'HKCU:\Software\Classes\DXFCanvasInstallerTest'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\DXFCanvasInstallerTest_is1'
$shortcut = [IO.Path]::GetFullPath((Join-Path ([Environment]::GetFolderPath('Programs')) 'DXF Canvas Installer Test/DXF Canvas Installer Test.lnk'))
$source = Join-Path $projectRoot 'dist/payload/dxf-canvas.exe'
$sourceHash = (Get-FileHash -LiteralPath $source).Hash
$version = (Get-Item -LiteralPath $source).VersionInfo.ProductVersion
& "$PSScriptRoot/build-installer.ps1" -Compiler $Compiler -TestIdentity -OutputDirectory 'test-output/installer/build'
$installer = Join-Path $root "build/DXF-Canvas-$version-setup-x64.exe"
try {
  foreach ($phase in @('install', 'repair')) {
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', "/DIR=`"$install`"", "/LOG=`"$root/$phase.log`"")
    $process = Start-Process -FilePath $installer -ArgumentList $arguments -PassThru -Wait -WindowStyle Hidden
    Assert-Check ($process.ExitCode -eq 0) "$phase : установщик завершился без ошибки"
    Assert-Check ((Get-FileHash -LiteralPath $exe).Hash -eq $sourceHash) "$phase : EXE совпадает с проверенной сборкой"
    Assert-Check ((Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -eq $version) "$phase : версия EXE верна"
    Assert-Check ((Get-ItemProperty $uninstallKey).DisplayVersion -eq $version) "$phase : версия в списке программ верна"
    Assert-Check (Test-Path -LiteralPath $shortcut) "$phase : создан ярлык меню Пуск"
    $target = [DxfShortcut]::Read($shortcut)
    # Shell может вернуть короткое имя 8.3: сравниваем оба пути через файловую систему.
    $filesystem = New-Object -ComObject Scripting.FileSystemObject
    [PSCustomObject]@{ expected=$exe; actual=$target; expected_short=$filesystem.GetFile($exe).ShortPath } |
      ConvertTo-Json | Set-Content -LiteralPath "$root/shortcut-$phase.json" -Encoding UTF8
    Assert-Check (Test-Path -LiteralPath $target) "$phase : цель ярлыка существует"
    Assert-Check ($filesystem.GetFile($target).ShortPath -eq $filesystem.GetFile($exe).ShortPath) "$phase : ярлык ведёт на постоянный путь"
    foreach ($extension in @('DXF', 'DWG')) {
      $command = (Get-Item "$registry.$extension\shell\open\command").GetValue('')
      Assert-Check ($command -eq "`"$exe`" `"%1`"") "$phase : $extension открывается с корректными кавычками"
      $progids = Get-Item "HKCU:\Software\Classes\.$($extension.ToLowerInvariant())\OpenWithProgids"
      Assert-Check ($progids.GetValueNames() -contains "DXFCanvasInstallerTest.$extension") "$phase : $extension доступен в Открыть с помощью"
      $association = (Get-Item 'HKCU:\Software\DXFCanvasInstallerTest\Capabilities\FileAssociations').GetValue(".$($extension.ToLowerInvariant())")
      Assert-Check ($association -eq "DXFCanvasInstallerTest.$extension") "$phase : $extension зарегистрирован в приложениях по умолчанию"
    }
    Assert-Check ((UserChoices) -eq $beforeChoices) "$phase : выбор приложений пользователя сохранён"
    if ($phase -eq 'install') {
      # Повторная установка обязана восстановить повреждённый файл приложения.
      [IO.File]::WriteAllText($exe, 'repair probe')
      [IO.File]::WriteAllText((Join-Path $install 'user-drawing.dxf'), 'user data')
    }
  }
  Assert-Check ((Get-Content -LiteralPath "$install/user-drawing.dxf" -Raw) -eq 'user data') 'Повторная установка сохранила посторонний файл'
  $process = Start-Process -FilePath $uninstaller -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$root/uninstall.log`"") -PassThru -Wait -WindowStyle Hidden
  Assert-Check ($process.ExitCode -eq 0) 'Удаление завершилось без ошибки'
  Assert-Check (!(Test-Path -LiteralPath $exe)) 'Удалён EXE'
  Assert-Check (!(Test-Path -LiteralPath $shortcut)) 'Удалён ярлык'
  Assert-Check (!(Test-Path $uninstallKey)) 'Удалена запись установленной программы'
  foreach ($extension in @('DXF', 'DWG')) {
    Assert-Check (!(Test-Path "$registry.$extension")) "Удалена регистрация $extension"
    $key = Get-Item "HKCU:\Software\Classes\.$($extension.ToLowerInvariant())\OpenWithProgids"
    Assert-Check (!($key.GetValueNames() -contains "DXFCanvasInstallerTest.$extension")) "Удалена запись Открыть с помощью для $extension"
  }
  Assert-Check (!(Test-Path 'HKCU:\Software\DXFCanvasInstallerTest\Capabilities')) 'Удалены возможности тестового приложения'
  Assert-Check ((UserChoices) -eq $beforeChoices) 'Удаление сохранило выбор приложений пользователя'
  Assert-Check (Test-Path -LiteralPath "$install/user-drawing.dxf") 'Удаление сохранило посторонний чертёж'
  $checks | ConvertTo-Json | Set-Content -LiteralPath "$root/results.json" -Encoding UTF8
  Write-Output "Проверок установщика пройдено: $($checks.Count)"
} finally {
  if (Test-Path -LiteralPath $exe) {
    if (Test-Path -LiteralPath $uninstaller) {
      Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -Wait -WindowStyle Hidden
    }
  }
}
