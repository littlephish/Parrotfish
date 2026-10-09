#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\dist\release\ps-app.exe"
#endif
#ifndef UpdaterExe
  #define UpdaterExe "..\target\updater\release\update.exe"
#endif
#ifndef NoticesFile
  #define NoticesFile "..\dist\THIRD-PARTY-NOTICES.txt"
#endif
#ifndef ReadmeFile
  #define ReadmeFile "..\README.md"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
AppId={{B21EECED-B833-438E-BA5E-C0C98D9278CD}
AppName=Parrotfish
AppVersion={#AppVersion}
AppVerName=Parrotfish {#AppVersion}
AppPublisher=Parrotfish project
DefaultDirName={autopf}\Parrotfish
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
OutputDir={#OutputDir}
OutputBaseFilename=Parrotfish-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\ps-app\ui\app-icon.ico
UninstallDisplayName=Parrotfish
UninstallDisplayIcon={app}\Parrotfish.exe
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
Type: files; Name: "{autoprograms}\PhishSpeak.lnk"; Check: EarlierProgramIsHere
Type: files; Name: "{autodesktop}\PhishSpeak.lnk"; Check: EarlierProgramIsHere
Type: files; Name: "{app}\PhishSpeak.exe"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "Parrotfish.exe"; Flags: ignoreversion
Source: "{#UpdaterExe}"; DestDir: "{app}"; DestName: "update.exe"; Flags: ignoreversion
Source: "{#NoticesFile}"; DestDir: "{app}"; DestName: "THIRD-PARTY-NOTICES.txt"; Flags: ignoreversion
Source: "{#ReadmeFile}"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Parrotfish"; Filename: "{app}\Parrotfish.exe"
Name: "{autodesktop}\Parrotfish"; Filename: "{app}\Parrotfish.exe"; Tasks: desktopicon

[UninstallDelete]
Type: files; Name: "{app}\update-log.txt"
Type: files; Name: "{app}\Parrotfish.exe.old*"
Type: files; Name: "{app}\PhishSpeak.exe.old*"
Type: files; Name: "{app}\update.exe.old*"
Type: files; Name: "{app}\README.md.old*"
Type: files; Name: "{app}\THIRD-PARTY-NOTICES.txt.old*"
Type: filesandordirs; Name: "{app}\update\unpacked"
Type: dirifempty; Name: "{app}\update"

[UninstallRun]
Filename: "{app}\Parrotfish.exe"; Parameters: "--forget-links"; RunOnceId: "ForgetLinks"; Flags: runhidden skipifdoesntexist

[Run]
Filename: "{app}\Parrotfish.exe"; Description: "{cm:LaunchProgram,Parrotfish}"; Flags: nowait postinstall skipifsilent

[Code]
function EarlierProgramIsHere(): Boolean;
begin
  try
    Result := FileExists(ExpandConstant('{app}\PhishSpeak.exe'));
  except
    Result := False;
  end;
end;
