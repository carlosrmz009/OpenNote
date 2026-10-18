; Inno Setup script for the Windows installer. Built by .github/workflows/release.yml:
;   iscc /DAppVersion=1.2.3 /DSourceDir=dist\opennote /DOutputDir=out packaging\windows\opennote.iss
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\opennote"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\out"
#endif

[Setup]
AppId={{6B0D6F0A-3B5E-4C1E-9E67-0E5A4F6E1C21}
AppName=OpenNote
AppVersion={#AppVersion}
AppPublisher=OpenNote
DefaultDirName={autopf}\OpenNote
DefaultGroupName=OpenNote
UninstallDisplayIcon={app}\opennote.exe
SetupIconFile=..\icon.ico
OutputDir={#OutputDir}
OutputBaseFilename=OpenNote-{#AppVersion}-windows-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Installs for the current user only, so no administrator password is asked for.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile={#SourceDir}\LICENSE

[Tasks]
Name: "desktopicon"; Description: "Put an OpenNote icon on the desktop"; GroupDescription: "Shortcuts:"
Name: "associate"; Description: "Open .musicxml, .mxl and MIDI files with OpenNote"; GroupDescription: "Files:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{group}\OpenNote"; Filename: "{app}\opennote.exe"; IconFilename: "{app}\icon.ico"
Name: "{autodesktop}\OpenNote"; Filename: "{app}\opennote.exe"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\OpenNote.Score"; ValueType: string; ValueData: "Score for OpenNote"; Flags: uninsdeletekey; Tasks: associate
Root: HKA; Subkey: "Software\Classes\OpenNote.Score\DefaultIcon"; ValueType: string; ValueData: "{app}\icon.ico"; Tasks: associate
Root: HKA; Subkey: "Software\Classes\OpenNote.Score\shell\open\command"; ValueType: string; ValueData: """{app}\opennote.exe"" ""%1"""; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.musicxml\OpenWithProgids"; ValueType: string; ValueName: "OpenNote.Score"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.mxl\OpenWithProgids"; ValueType: string; ValueName: "OpenNote.Score"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.mid\OpenWithProgids"; ValueType: string; ValueName: "OpenNote.Score"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.midi\OpenWithProgids"; ValueType: string; ValueName: "OpenNote.Score"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate

[Run]
Filename: "{app}\opennote.exe"; Description: "Start OpenNote"; Flags: nowait postinstall skipifsilent
