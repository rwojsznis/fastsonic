; The Windows installer, built with Inno Setup 6.3 or later from a release
; binary (the release workflow does this on every tag):
;
;   iscc /DVersion=0.1.4 /DArch=x86_64 /DBinary=...\fastpotify.exe ^
;        /DOutputDir=dist packaging\windows\fastpotify.iss
;
; Arch is x86_64 or aarch64, as in the Rust target triple, so the installer
; is named like the zip next to it. It needs no administrator rights: the
; program goes to the user's own Programs folder with a Start menu entry,
; and a running copy is closed before an update replaces it.

#ifndef Version
  #error Version must be defined on the ISCC command line
#endif
#ifndef Arch
  #error Arch must be defined on the ISCC command line (x86_64 or aarch64)
#endif
#ifndef Binary
  #error Binary must be defined on the ISCC command line
#endif
#ifndef OutputDir
  #error OutputDir must be defined on the ISCC command line
#endif
#if Arch == "aarch64"
  #define InnoArch "arm64"
#else
  #define InnoArch "x64compatible"
#endif

#define AppName "Fastpotify"
#define AppExeName "fastpotify.exe"

[Setup]
; Never change: this is how Windows tells an update from a new program.
AppId={{FCED1EA0-EBF5-4C32-BA3B-A3AD724BACC3}
AppName={#AppName}
AppVersion={#Version}
AppVerName={#AppName} {#Version}
AppPublisher=Carmine Paolino
AppPublisherURL=https://fastpotify.rocks
AppSupportURL=https://github.com/crmne/fastpotify/issues
AppUpdatesURL=https://fastpotify.rocks/download/
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed={#InnoArch}
ArchitecturesInstallIn64BitMode={#InnoArch}
MinVersion=10.0
LicenseFile=..\..\LICENSE
OutputDir={#OutputDir}
OutputBaseFilename=fastpotify-v{#Version}-{#Arch}-pc-windows-msvc-setup
SetupIconFile=fastpotify.ico
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\{#AppExeName}
VersionInfoVersion={#Version}.0

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#Binary}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent
