; The Orbit installer for Windows (Inno Setup 6, preinstalled on the windows-2025 GitHub runner).
; Per user: no admin rights, installs to %LOCALAPPDATA%\Programs\Orbit, Start-menu shortcut, an
; optional desktop shortcut, and an uninstaller in Settings -> Apps.
; The shortcuts carry the AppUserModelID that the app sets on its process (src/app_id.rs): with it,
; Windows names "Orbit" as the sender of its toasts.
;
; .github/workflows/desktop.yml runs, in apps/desktop/packaging/windows:
;   ISCC.exe /DAppVersion=<version> /DSourceExe=<path to Orbit.exe> /O<output folder> orbit.iss

#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef SourceExe
  #error Pass /DSourceExe=<path to Orbit.exe>
#endif

[Setup]
; Never change AppId: an update finds the installed app (and its folder) by it.
AppId={{5ABA2EAC-A9D3-4DEA-AAD8-CAF261D9FDF1}
AppName=Orbit
AppVersion={#AppVersion}
AppVerName=Orbit {#AppVersion}
AppPublisher=Wellrox
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
DefaultDirName={autopf}\Orbit
DisableDirPage=yes
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=orbit.ico
UninstallDisplayIcon={app}\Orbit.exe
UninstallDisplayName=Orbit
OutputBaseFilename=Orbit-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; A running Orbit (also hidden in the tray) holds Orbit.exe: close it for an update or uninstall.
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "Orbit.exe"; Flags: ignoreversion
Source: "README.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Orbit"; Filename: "{app}\Orbit.exe"; AppUserModelID: "Wellrox.Orbit"
Name: "{autodesktop}\Orbit"; Filename: "{app}\Orbit.exe"; AppUserModelID: "Wellrox.Orbit"; Tasks: desktopicon

[Registry]
; "Start with Windows" is the app's own setting (the tray menu writes this value); the uninstaller
; removes it so Windows does not try to start a removed app.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Orbit"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\Orbit.exe"; Description: "Start Orbit"; Flags: nowait postinstall skipifsilent
