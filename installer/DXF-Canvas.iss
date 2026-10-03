#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef PayloadDir
  #error PayloadDir is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif
#ifndef AppIdValue
  #define AppIdValue "{{8FCED2BC-4713-445A-9A07-BFA5465DE42C}"
#endif
#ifndef AppName
  #define AppName "DXF Холст"
#endif
#ifndef RegistryName
  #define RegistryName "DXFCanvas"
#endif
#ifndef InstallFolder
  #define InstallFolder "DXF Canvas"
#endif

[Setup]
AppId={#AppIdValue}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=BrianIS8090
AppPublisherURL=https://github.com/BrianIS8090/dxf-canvas
AppSupportURL=https://github.com/BrianIS8090/dxf-canvas/issues
AppUpdatesURL=https://github.com/BrianIS8090/dxf-canvas/releases
DefaultDirName={localappdata}\Programs\{#InstallFolder}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=DXF-Canvas-{#AppVersion}-setup-x64
UninstallDisplayIcon={app}\dxf-canvas.exe
UninstallDisplayName={#AppName} {#AppVersion}
VersionInfoVersion={#AppVersion}.0
VersionInfoProductName=DXF Canvas
VersionInfoDescription=DXF Canvas Setup
ChangesAssociations=yes
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
SetupLogging=yes
LicenseFile={#PayloadDir}\LICENSE.txt
#ifdef SignedBuild
SignTool=dxf
SignedUninstaller=yes
#endif

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Создать ярлык на рабочем столе"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\dxf-canvas.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\dxf-canvas.exe"; Comment: "{#AppName} {#AppVersion} — просмотр DXF и DWG"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\dxf-canvas.exe"; Tasks: desktopicon; Comment: "{#AppName} {#AppVersion}"

[Registry]
Root: HKCU; Subkey: "Software\Classes\{#RegistryName}.DXF"; ValueType: string; ValueData: "Чертёж DXF"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\{#RegistryName}.DXF\DefaultIcon"; ValueType: string; ValueData: """{app}\dxf-canvas.exe"",0"
Root: HKCU; Subkey: "Software\Classes\{#RegistryName}.DXF\shell\open\command"; ValueType: string; ValueData: """{app}\dxf-canvas.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\.dxf\OpenWithProgids"; ValueName: "{#RegistryName}.DXF"; ValueType: none; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\{#RegistryName}.DWG"; ValueType: string; ValueData: "Чертёж DWG"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\{#RegistryName}.DWG\DefaultIcon"; ValueType: string; ValueData: """{app}\dxf-canvas.exe"",0"
Root: HKCU; Subkey: "Software\Classes\{#RegistryName}.DWG\shell\open\command"; ValueType: string; ValueData: """{app}\dxf-canvas.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\.dwg\OpenWithProgids"; ValueName: "{#RegistryName}.DWG"; ValueType: none; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\{#RegistryName}\Capabilities"; ValueName: "ApplicationName"; ValueType: string; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\{#RegistryName}\Capabilities"; ValueName: "ApplicationDescription"; ValueType: string; ValueData: "Просмотр DXF и DWG, слои и измерения"
Root: HKCU; Subkey: "Software\{#RegistryName}\Capabilities"; ValueName: "ApplicationIcon"; ValueType: string; ValueData: """{app}\dxf-canvas.exe"",0"
Root: HKCU; Subkey: "Software\{#RegistryName}\Capabilities\FileAssociations"; ValueName: ".dxf"; ValueType: string; ValueData: "{#RegistryName}.DXF"
Root: HKCU; Subkey: "Software\{#RegistryName}\Capabilities\FileAssociations"; ValueName: ".dwg"; ValueType: string; ValueData: "{#RegistryName}.DWG"
Root: HKCU; Subkey: "Software\RegisteredApplications"; ValueName: "{#RegistryName}"; ValueType: string; ValueData: "Software\{#RegistryName}\Capabilities"; Flags: uninsdeletevalue

[Run]
Filename: "ms-settings:defaultapps"; Description: "Открыть выбор приложений по умолчанию для DXF и DWG"; Flags: shellexec postinstall skipifsilent unchecked
Filename: "{app}\dxf-canvas.exe"; Description: "Запустить {#AppName}"; Flags: nowait postinstall skipifsilent unchecked

[Code]
function ApplicationRunning(): Boolean;
var
  Locator, Services, Processes: Variant;
begin
  { Не закрываем холст автоматически: раскладка и измерения живут в памяти. }
  Result := True;
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Services := Locator.ConnectServer('', 'root\CIMV2');
    Processes := Services.ExecQuery('SELECT ProcessId FROM Win32_Process WHERE Name = ''dxf-canvas.exe''');
    Result := Processes.Count > 0;
  except
    Log('Не удалось проверить запущенные окна DXF Холст');
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if ApplicationRunning() then
    Result := 'Завершите работу и закройте все окна DXF Холст, затем повторите установку. Расстановка и измерения текущего окна не сохраняются при закрытии.';
end;

function InitializeUninstall(): Boolean;
begin
  Result := not ApplicationRunning();
  if not Result and not UninstallSilent then
    MsgBox('Перед удалением завершите работу и закройте все окна DXF Холст.', mbInformation, MB_OK);
end;
