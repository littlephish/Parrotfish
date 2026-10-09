#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\dist\release\ps-app.exe"
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
Source: "{#NoticesFile}"; DestDir: "{app}"; DestName: "THIRD-PARTY-NOTICES.txt"; Flags: ignoreversion
Source: "{#ReadmeFile}"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Parrotfish"; Filename: "{app}\Parrotfish.exe"
Name: "{autodesktop}\Parrotfish"; Filename: "{app}\Parrotfish.exe"; Tasks: desktopicon

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
