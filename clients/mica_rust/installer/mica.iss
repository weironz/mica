; Inno Setup script for the Mica pure-Rust Windows desktop client.
; Bundles the standalone native Rust release executable into a per-user
; installer (no admin/UAC). Compile with ISCC.exe.

#define AppName "Mica"
#ifndef AppVersion
  #define AppVersion "0.15.2"
#endif
#define AppPublisher "weironz"
#define AppExe "mica-rust.exe"
#define SourceDir "..\target\release"

[Setup]
AppId={{B2C3A1D4-5E6F-4789-A0B1-C2D3E4F50617}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\{#AppName}
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
SetupIconFile=..\assets\app_icon.ico
OutputDir=Output
OutputBaseFilename=Mica-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
CreateUninstallRegKey=not IsSmokeTest

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; Clean up legacy Flutter runtime files left behind when upgrading in-place from
; <= 0.14.x Flutter installs (same AppId and {app} directory). Pure-Rust Mica
; ships a single standalone mica-rust.exe with zero DLLs or data\ directory.
Type: files; Name: "{app}\mica_flutter.exe"
Type: files; Name: "{app}\*.dll"
Type: files; Name: "{app}\native_assets.json"
Type: filesandordirs; Name: "{app}\data"

[Files]
Source: "{#SourceDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Check: not IsSmokeTest
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon; Check: not IsSmokeTest

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall; Check: not CmdLineParamExists('/SKIPRUN') and not IsSmokeTest

[Code]
const
  SYNCHRONIZE = $00100000;
  WAIT_TIMEOUT = $00000102;

function OpenProcess(dwDesiredAccess: DWORD; bInheritHandle: BOOL; dwProcessId: DWORD): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function WaitForSingleObject(hHandle: THandle; dwMilliseconds: DWORD): DWORD;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(hObject: THandle): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';

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
    Exit;
  if WaitForSingleObject(H, 60000) = WAIT_TIMEOUT then
    Result := 'Mica is still running. Close it, then run the update again.';
  CloseHandle(H);
end;
