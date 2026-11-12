; Per-user ownership is deliberately limited to this explicit payload.
#ifndef PayloadDir
  #error PayloadDir must name the staged release payload
#endif
#ifndef PackageVersion
  #error PackageVersion must match the reviewed release version
#endif

[Setup]
AppId={{D2286755-6B43-49F1-AC97-09151B20B0A8}
AppName=WiniOS
AppVersion={#PackageVersion}
AppVerName=WiniOS {#PackageVersion} private beta
AppPublisher=Abhishyanth
DefaultDirName={localappdata}\Programs\WiniOS
UsePreviousAppDir=no
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
AllowNetworkDrive=no
AllowUNCPath=no
AllowRootDirectory=no
WizardStyle=modern dark
SetupIconFile={#PayloadDir}\winios.ico
UninstallDisplayIcon={app}\winios.exe
UninstallDisplayName=WiniOS
Uninstallable=yes
UninstallLogMode=append
OutputBaseFilename=WiniOS-Setup
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=no
RestartApplications=no
RestartIfNeededByRun=no
SetupLogging=no
InfoBeforeFile={#PayloadDir}\INSTALLATION.txt
VersionInfoVersion={#PackageVersion}
VersionInfoCompany=Abhishyanth
VersionInfoDescription=WiniOS per-user installer
VersionInfoProductName=WiniOS

[Files]
Source: "{#PayloadDir}\winios.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\winios-agent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\winios.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\INSTALLATION.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\README.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#PayloadDir}\mission-010.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#PayloadDir}\mission-014.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#PayloadDir}\mission-015.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#PayloadDir}\mission-018.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#PayloadDir}\private-beta.md"; DestDir: "{app}\docs"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\WiniOS"; Filename: "{app}\winios.exe"; Parameters: "--tray"; WorkingDir: "{app}"; IconFilename: "{app}\winios.ico"; AppUserModelID: "WiniOS.Desktop"

[Run]
Filename: "{app}\winios.exe"; Parameters: "--tray"; WorkingDir: "{app}"; Description: "Launch WiniOS"; Flags: nowait postinstall skipifsilent
Filename: "{app}\winios.exe"; Parameters: "--tray"; WorkingDir: "{app}"; Flags: nowait; Check: LaunchSilent

[Messages]
ConfirmUninstall=Remove %1 and its packaged files, shortcut and WiniOS startup registration?%n%nYour settings, Credential Manager token and TLS files will be kept.%n%nThe user-managed Tailscale Serve route may remain. WiniOS will not change Serve, firewall or certificates. Review "tailscale serve status" yourself. Only for the dedicated WiniOS HTTPS 443 listener, after approval, "tailscale serve --https=443 off" removes that listener. Do not reset unrelated routes.
UninstalledAll=%1 was removed. Settings, credentials and TLS were preserved.%n%nA Tailscale Serve route may still point at the stopped localhost backend. Review "tailscale serve status". For the dedicated WiniOS listener only, manually run "tailscale serve --https=443 off" after approval. No network settings were changed.

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  AgentMutex = 'Local\WiniOS.Agent';
  MaintenanceMutex = 'Local\WiniOS.Maintenance';
  AlreadyExists = 183;
  WM_CLOSE = $0010;
var
  MaintenanceHandle, AgentHandle: THandle;
  StartupOn: Boolean;
  StartupOriginal: String;

function NativeCreateMutex(Security: NativeInt; InitialOwner: Boolean; Name: String): THandle;
  external 'CreateMutexW@kernel32.dll stdcall';
function NativeCloseHandle(Handle: THandle): Boolean;
  external 'CloseHandle@kernel32.dll stdcall';
function NativeLastError: Cardinal;
  external 'GetLastError@kernel32.dll stdcall';
function NativePostMessage(Window: HWND; Msg: Cardinal; WParam, LParam: NativeInt): Boolean;
  external 'PostMessageW@user32.dll stdcall';
function NativeWindowProcessId(Window: HWND; var ProcessId: Cardinal): Cardinal;
  external 'GetWindowThreadProcessId@user32.dll stdcall';
function NativeOpenProcess(Access: Cardinal; Inherit: Boolean; ProcessId: Cardinal): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function NativeWaitForSingleObject(Handle: THandle; Milliseconds: Cardinal): Cardinal;
  external 'WaitForSingleObject@kernel32.dll stdcall';

procedure ReleaseAgent;
begin
  if AgentHandle <> 0 then begin
    NativeCloseHandle(AgentHandle);
    AgentHandle := 0;
  end;
end;

function EnterMaintenance: Boolean;
var Error: Cardinal;
begin
  MaintenanceHandle := NativeCreateMutex(0, False, MaintenanceMutex);
  Error := NativeLastError;
  Result := (MaintenanceHandle <> 0) and (Error <> AlreadyExists);
  if not Result then begin
    if MaintenanceHandle <> 0 then NativeCloseHandle(MaintenanceHandle);
    MaintenanceHandle := 0;
    SuppressibleMsgBox('Another WiniOS installation or uninstall is active. Close it and retry.', mbError, MB_OK, IDOK);
  end;
end;

procedure LeaveMaintenance;
begin
  ReleaseAgent;
  if MaintenanceHandle <> 0 then begin
    NativeCloseHandle(MaintenanceHandle);
    MaintenanceHandle := 0;
  end;
end;

function FixedPath: Boolean;
begin
  Result := CompareText(RemoveBackslashUnlessRoot(ExpandFileName(ExpandConstant('{app}'))),
    RemoveBackslashUnlessRoot(ExpandFileName(ExpandConstant('{localappdata}\Programs\WiniOS')))) = 0;
end;

function ManagedStartup(Command: String): Boolean;
var Suffix, Image: String; Count: Integer;
begin
  Result := False;
  Suffix := '" --startup';
  Count := Length(Command);
  if (Count < 18) or (Count >= 260) then Exit;
  if (Command[1] <> '"') or (Copy(Command, Count-Length(Suffix)+1, Length(Suffix)) <> Suffix) then Exit;
  Image := Copy(Command, 2, Count-Length(Suffix)-1);
  if (Pos('"', Image) <> 0) or (Pos(#13, Image) <> 0) or (Pos(#10, Image) <> 0) then Exit;
  Result := (Length(Image) > 3) and (Image[2] = ':') and (Image[3] = '\') and
    (CompareText(ExtractFileName(Image), 'winios.exe') = 0);
end;

function SnapshotStartup: Boolean;
begin
  StartupOn := False;
  StartupOriginal := '';
  Result := True;
  if not RegValueExists(HKCU, RunKey, 'WiniOS') then Exit;
  Result := RegQueryStringValue(HKCU, RunKey, 'WiniOS', StartupOriginal) and ManagedStartup(StartupOriginal);
  StartupOn := Result;
end;

function StopAndLockAgent: Boolean;
var Window: HWND; ProcessHandle: THandle; ProcessId, Error: Cardinal;
begin
  Result := False;
  if AgentHandle <> 0 then begin Result := True; Exit; end;
  if CheckForMutexes(AgentMutex) then begin
    Window := FindWindowByClassName('WiniOSTrayWindow');
    if Window = 0 then begin
      Log('WiniOS console/unknown instance is running; automatic shutdown refused.');
      Exit;
    end;
    NativeWindowProcessId(Window, ProcessId);
    if ProcessId = 0 then Exit;
    ProcessHandle := NativeOpenProcess($00100000, False, ProcessId); { SYNCHRONIZE only }
    if ProcessHandle = 0 then Exit;
    try
      if not NativePostMessage(Window, WM_CLOSE, 0, 0) then Exit;
      if NativeWaitForSingleObject(ProcessHandle, 20000) <> 0 then Exit;
    finally
      NativeCloseHandle(ProcessHandle);
    end;
    if CheckForMutexes(AgentMutex) then Exit;
  end;
  { Atomically take the SAME mutex checked by every existing GUI/CLI binary.
    A launch racing shutdown makes this fail before any file is replaced. }
  AgentHandle := NativeCreateMutex(0, False, AgentMutex);
  Error := NativeLastError;
  Result := (AgentHandle <> 0) and (Error <> AlreadyExists);
  if not Result then ReleaseAgent;
end;

function InitializeSetup: Boolean;
begin
  Result := EnterMaintenance;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if not FixedPath then begin
    Result := 'WiniOS uses the fixed per-user LocalAppData\Programs\WiniOS directory. A different /DIR is not allowed.';
    Exit;
  end;
  if not StopAndLockAgent then begin
    Result := 'WiniOS did not stop safely. Quit from the tray, or press Ctrl+C in the console agent, and retry. No process was terminated and no files were replaced.';
    Exit;
  end;
  if not SnapshotStartup then begin
    ReleaseAgent;
    Result := 'An unrecognized WiniOS startup value exists. Review it in Windows Startup Apps before retrying; it was not overwritten.';
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var Current, Expected: String;
begin
  if CurStep = ssPostInstall then begin
    if StartupOn then begin
      if not RegQueryStringValue(HKCU, RunKey, 'WiniOS', Current) or (Current <> StartupOriginal) then
        RaiseException('Startup registration changed during installation; it was not overwritten.');
      Expected := '"' + ExpandConstant('{app}\winios.exe') + '" --startup';
      if (Current <> Expected) and not RegWriteStringValue(HKCU, RunKey, 'WiniOS', Expected) then
        RaiseException('Could not repair existing startup registration. Review Windows Startup Apps.');
    end;
    ReleaseAgent;
  end;
end;

function LaunchSilent: Boolean;
begin
  { Inno may evaluate Check before ssPostInstall. It executes [Run] only
    after successful installation; never gate this on a late mutable flag. }
  Result := WizardSilent and (ExpandConstant('{param:LAUNCH|0}') = '1');
end;

procedure DeinitializeSetup;
begin
  LeaveMaintenance;
end;

function InitializeUninstall: Boolean;
begin
  Result := FixedPath and EnterMaintenance;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var Command: String;
begin
  if CurUninstallStep = usUninstall then begin
    if not StopAndLockAgent then
      RaiseException('WiniOS could not stop safely. Quit it first and retry uninstall. No thread or process was terminated.');
    if RegQueryStringValue(HKCU, RunKey, 'WiniOS', Command) then begin
      if ManagedStartup(Command) then begin
        if not RegDeleteValue(HKCU, RunKey, 'WiniOS') then
          RaiseException('Could not remove the WiniOS startup registration. Uninstall aborted.');
      end else begin
        Log('Unrecognized startup command left untouched; review Windows Startup Apps.');
        SuppressibleMsgBox('An unrecognized WiniOS-named startup command was retained. Review it in Windows Startup Apps; the uninstaller does not delete another command.', mbInformation, MB_OK, IDOK);
      end;
    end;
  end;
  if CurUninstallStep = usPostUninstall then ReleaseAgent;
end;

procedure DeinitializeUninstall;
begin
  LeaveMaintenance;
end;
