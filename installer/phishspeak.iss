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
AppName=PhishSpeak
AppVersion={#AppVersion}
AppVerName=PhishSpeak {#AppVersion}
AppPublisher=PhishSpeak project
DefaultDirName={autopf}\PhishSpeak
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
OutputDir={#OutputDir}
OutputBaseFilename=PhishSpeak-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=PhishSpeak
UninstallDisplayIcon={app}\PhishSpeak.exe
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "PhishSpeak.exe"; Flags: ignoreversion
Source: "{#NoticesFile}"; DestDir: "{app}"; DestName: "THIRD-PARTY-NOTICES.txt"; Flags: ignoreversion
Source: "{#ReadmeFile}"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\PhishSpeak"; Filename: "{app}\PhishSpeak.exe"
Name: "{autodesktop}\PhishSpeak"; Filename: "{app}\PhishSpeak.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\PhishSpeak.exe"; Description: "{cm:LaunchProgram,PhishSpeak}"; Flags: nowait postinstall skipifsilent
