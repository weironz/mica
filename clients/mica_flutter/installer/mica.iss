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
; A smoke-test install (/MICASMOKE, see IsSmokeTest below) shares this AppId
; with the user's real install, so it must not write the HKCU uninstall entry:
; it would repoint "Apps & features" at the throwaway directory, and the smoke
; test's own uninstall would then delete the real install's entry (#179).
; unins000.exe is still written, so the test can uninstall what it installed.
CreateUninstallRegKey=not IsSmokeTest

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
; Same path as the real install's shortcuts: a smoke-test install would
; repoint them at its throwaway directory and its uninstall would delete them.
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Check: not IsSmokeTest
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon; Check: not IsSmokeTest

[Run]
; No `skipifsilent`: the in-app updater installs with /VERYSILENT and relies on
; this step to relaunch Mica afterwards. In an interactive install it is still
; the usual "launch now" checkbox.
;
; `Check: not CmdLineParamExists('/SKIPRUN')` lets a caller suppress JUST this
; auto-launch while keeping the silent install — the CI installer smoke-test
; passes /SKIPRUN so IT becomes the sole launcher of the installed app. Without
; that, the installer auto-launches one instance AND the test launched a second,
; and the app's single-instance mutex (windows/runner/main.cpp) made whichever
; lost the race exit 0 — a flaky "exited within 10s" failure. The updater never
; passes /SKIPRUN, so its relaunch is unchanged. (Pattern: ShareX's /NORUN.)
; /MICASMOKE implies /SKIPRUN.
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall; Check: not CmdLineParamExists('/SKIPRUN') and not IsSmokeTest

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

{ True if Setup was invoked with the exact switch `Value` (e.g. '/SKIPRUN').
  Used by the [Run] Check to let CI suppress the post-install auto-launch — the
  canonical Inno idiom (see ShareX's /NORUN). }
function CmdLineParamExists(const Value: String): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), Value) = 0 then
    begin
      Result := True;
      Exit;
    end;
end;

{ True for the release smoke test's throwaway install (scripts/smoke-installer.ps1
  passes /MICASMOKE): no shortcuts, no uninstall registry entry, no auto-launch,
  so installing and uninstalling it leaves the user's real install untouched. }
function IsSmokeTest(): Boolean;
begin
  Result := CmdLineParamExists('/MICASMOKE');
end;

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
