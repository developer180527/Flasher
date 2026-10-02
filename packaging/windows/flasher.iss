; Flasher's Windows installer, for Inno Setup 6. Built by build.ps1, which
; passes the version and the exe to package:
;   ISCC /DAppVersion=0.1.0 /DSourceExe=..\..\target\release\flasher.exe flasher.iss
; Output: target\dist\Flasher-<version>-windows-x64-setup.exe

#ifndef AppVersion
  #error Pass the version: /DAppVersion=x.y.z
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\flasher.exe"
#endif

[Setup]
; Identifies Flasher to Windows across versions: never change it.
AppId={{EF8952B3-D2CA-45B9-B195-3E8ABC5C4594}
AppName=Flasher
AppVersion={#AppVersion}
AppVerName=Flasher {#AppVersion}
AppPublisher=Venu Gopal
AppPublisherURL=https://github.com/developer180527/Flasher
AppSupportURL=https://github.com/developer180527/Flasher/issues
AppUpdatesURL=https://github.com/developer180527/Flasher/releases
VersionInfoVersion={#AppVersion}
VersionInfoDescription=Flasher installer
; For everyone by default (Program Files, needs an administrator, as writing
; disks does anyway); the first page offers "only for me" instead.
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\Flasher
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\..\target\dist
OutputBaseFilename=Flasher-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\icons\flasher.ico
UninstallDisplayIcon={app}\flasher.exe
UninstallDisplayName=Flasher
WizardStyle=modern
; 100 % and 200 % display scaling; Setup picks the one that fits.
WizardImageFile=wizard-large-100.bmp,wizard-large-200.bmp
WizardSmallImageFile=wizard-small-100.bmp,wizard-small-200.bmp
Compression=lzma2/ultra64
SolidCompression=yes
; Close a running Flasher before replacing it.
CloseApplications=yes
; The PATH task changes the environment; tell running programs.
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addtopath"; Description: "Add Flasher to PATH, for terminal commands (flasher list, flasher write, ...)"; GroupDescription: "Terminal:"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\Inter-OFL.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Flasher"; Filename: "{app}\flasher.exe"; Comment: "Write disk images to USB drives and SD cards"
Name: "{autodesktop}\Flasher"; Filename: "{app}\flasher.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\flasher.exe"; Description: "{cm:LaunchProgram,Flasher}"; Flags: nowait postinstall skipifsilent

[Code]
// PATH: the machine's for an install for everyone, the user's for one
// "only for me". Added after installing when the task is ticked; removed
// on uninstall either way (a no-op if it was never added).

function EnvRoot: Integer;
begin
  if IsAdminInstallMode then
    Result := HKEY_LOCAL_MACHINE
  else
    Result := HKEY_CURRENT_USER;
end;

function EnvKey: String;
begin
  if IsAdminInstallMode then
    Result := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  else
    Result := 'Environment';
end;

procedure AddToPath;
var
  Path, Dir: String;
begin
  Dir := ExpandConstant('{app}');
  if not RegQueryStringValue(EnvRoot, EnvKey, 'Path', Path) then
    Path := '';
  if Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') > 0 then
    exit;
  if (Path <> '') and (Copy(Path, Length(Path), 1) <> ';') then
    Path := Path + ';';
  RegWriteExpandStringValue(EnvRoot, EnvKey, 'Path', Path + Dir);
end;

procedure RemoveFromPath;
var
  Path, Dir, Rest, Part, Kept: String;
  P: Integer;
  Found: Boolean;
begin
  if not RegQueryStringValue(EnvRoot, EnvKey, 'Path', Path) then
    exit;
  Dir := Uppercase(ExpandConstant('{app}'));
  Rest := Path;
  Kept := '';
  Found := False;
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    if P = 0 then
    begin
      Part := Rest;
      Rest := '';
    end
    else
    begin
      Part := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, Length(Rest));
    end;
    if Uppercase(Part) = Dir then
      Found := True
    else if Part <> '' then
    begin
      if Kept <> '' then
        Kept := Kept + ';';
      Kept := Kept + Part;
    end;
  end;
  if Found then
    RegWriteExpandStringValue(EnvRoot, EnvKey, 'Path', Kept);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddToPath;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath;
end;
