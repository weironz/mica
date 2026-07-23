; Inno Setup script for the Mica Windows desktop client.
; Bundles the entire Flutter release output (exe + DLLs + data/) into a
; per-user installer (no admin/UAC). Compile with ISCC.exe.

#define AppName "Mica"
; Version can be overridden from the command line: ISCC /DAppVersion=1.2.3
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppPublisher "weironz"
#define AppExe "mica_flutter.exe"
#define SourceDir "..\build\windows\x64\runner\Release"

[Setup]
AppId={{B2C3A1D4-5E6F-4789-A0B1-C2D3E4F50617}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\{#AppName}
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
OutputDir=Output
OutputBaseFilename=Mica-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
DisableProgramGroupPage=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
; No `skipifsilent`: the in-app updater installs with /VERYSILENT and relies on
; this step to relaunch Mica afterwards. In an interactive install it is still
; the usual "launch now" checkbox.
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall

[Code]
{ The in-app updater launches this Setup with /MICAWAITPID=<pid> and then exits
  ITSELF. It must: the app intercepts its own WM_CLOSE (close-to-tray), so
  RestartManager (/CLOSEAPPLICATIONS) can't close it — the app has to go first.
  We wait for that PID to disappear before copying a single file, so the update
  never races the app's file locks. This is the native replacement for the old
  external `ping` delay + its console window: no shell, no window, and we wait
  for the ACTUAL exit, not a guessed 3 s. Defensive throughout — if anything is
  off (no PID, OpenProcess fails), we simply proceed, never worse than before. }

const
  SYNCHRONIZE = $00100000;
  WAIT_TIMEOUT = $00000102;

function OpenProcess(dwDesiredAccess: DWORD; bInheritHandle: BOOL; dwProcessId: DWORD): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function WaitForSingleObject(hHandle: THandle; dwMilliseconds: DWORD): DWORD;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(hObject: THandle): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';

function MicaWaitPid(): DWORD;
var
  I: Integer;
  P, Prefix: String;
begin
  Result := 0;
  Prefix := '/MICAWAITPID=';
  for I := 1 to ParamCount do
  begin
    P := ParamStr(I);
    if Copy(P, 1, Length(Prefix)) = Prefix then
    begin
      Result := StrToIntDef(Copy(P, Length(Prefix) + 1, MaxInt), 0);
      Exit;
    end;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Pid: DWORD;
  H: THandle;
begin
  Result := '';
  Pid := MicaWaitPid();
  if Pid = 0 then
    Exit;
  H := OpenProcess(SYNCHRONIZE, False, Pid);
  if H = 0 then
    Exit;  { already gone, or no access — safe to proceed }
  { Block up to 60 s for the app to exit. It exit(0)s immediately, so this
    returns in milliseconds in practice; WAIT_TIMEOUT means it is still alive. }
  if WaitForSingleObject(H, 60000) = WAIT_TIMEOUT then
    Result := 'Mica is still running. Close it, then run the update again.';
  CloseHandle(H);
end;
