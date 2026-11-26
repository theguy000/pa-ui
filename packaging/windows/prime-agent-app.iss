; Inno Setup script for Prime Agent (Windows x64, per-user install).
; Invoked by scripts/package-windows.ps1 with /DMyAppVersion, /DSourceDir, /DOutputDir, /DIconPath.

#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\stage\prime-agent-app"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif
#ifndef IconPath
  #define IconPath "..\..\crates\pa-ui\assets\icon.ico"
#endif

[Setup]
AppId={{7A9E4B1E-2C4B-4F7D-9D2B-3F4E1B8A5C12}
AppName=Prime Agent
AppVersion={#MyAppVersion}
AppVerName=Prime Agent {#MyAppVersion}
AppPublisher=Prime Agent
AppPublisherURL=https://github.com/theguy000/prime-agent-app
AppSupportURL=https://github.com/theguy000/prime-agent-app/issues
DefaultDirName={localappdata}\Programs\Prime Agent
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
#if VER >= EncodeVer(6, 7)
  ; Per-user install (PrivilegesRequired=lowest): disable RedirectionGuard so the
  ; post-install prime-agent-app.exe and its long-lived pa-daemon.exe do not inherit
  ; Windows ProcessRedirectionTrustPolicy, which blocks user-created NTFS junctions
  ; (such as uv-managed Python interpreters).
  RedirectionGuard=no
#endif
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
CloseApplicationsFilter=*.exe
RestartApplications=no
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
SetupIconFile={#IconPath}
UninstallDisplayIcon={app}\prime-agent-app.exe
UninstallDisplayName=Prime Agent
OutputDir={#OutputDir}
OutputBaseFilename=PrimeAgent-Setup-{#MyAppVersion}-windows-x64

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[InstallDelete]
Type: filesandordirs; Name: "{app}\prime-agent-runtime"
Type: filesandordirs; Name: "{app}\skills"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Prime Agent"; Filename: "{app}\prime-agent-app.exe"
Name: "{autodesktop}\Prime Agent"; Filename: "{app}\prime-agent-app.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\prime-agent-app.exe"; Description: "{cm:LaunchProgram,Prime Agent}"; Flags: nowait postinstall skipifsilent

[Code]
procedure StopRunningPrimeAgent();
var
  UiRc, DaemonRc: Integer;
begin
  UiRc := -1;
  DaemonRc := -1;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM prime-agent-app.exe', '', SW_HIDE, ewWaitUntilTerminated, UiRc);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM pa-daemon.exe', '', SW_HIDE, ewWaitUntilTerminated, DaemonRc);
  if (UiRc = 0) or (DaemonRc = 0) then
    Sleep(400);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningPrimeAgent();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    StopRunningPrimeAgent();
end;
