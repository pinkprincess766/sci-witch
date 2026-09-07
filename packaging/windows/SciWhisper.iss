; Inno Setup script for the SciWhisper Windows installer.
;
; Two decisions here are not cosmetic and are pinned by
; `crates/sciwhisper-update/tests/installer.rs`:
;
; 1. The install is **per-user**, into %LOCALAPPDATA%\Programs\SciWhisper.
;    An admin install into Program Files would mean the updater needs an
;    elevation prompt for every single update, and a program that asks for
;    administrator rights routinely is a program whose users stop reading the
;    prompt. Per-user also means no elevation for the install itself.
;
; 2. Nothing starts by itself. Auto-start at login is offered as an
;    **unchecked** task, and no update check runs on a schedule. A dictation
;    tool that launches itself and reaches the network on its own has broken
;    the promise it was chosen for.
;
; The installer is not code-signed. SmartScreen will warn, and the warning is
; correct: nobody has vouched for these bytes. See README-WINDOWS.txt.

#define AppName "SciWhisper"
#define AppPublisher "SciWhisper contributors"
#define AppUrl "https://github.com/pinkprincess766/sci-witch"
; Passed in by the release workflow: ISCC /DAppVersion=0.1.1-rc1 /DSourceDir=...
#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef SourceDir
  #define SourceDir "bundle"
#endif

[Setup]
AppId={{7C2E6F0A-4B1D-4F5E-9C3A-5E7D1B8A2F44}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
VersionInfoVersion=0.1.1
; Per-user: no elevation, and the updater can replace the directory without a
; UAC prompt.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputBaseFilename=SciWhisper-{#AppVersion}-Windows-x64-Setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile={#SourceDir}\SciWhisper.ico
UninstallDisplayIcon={app}\sciwhisper.exe
WizardStyle=modern
LicenseFile={#SourceDir}\LICENSE
InfoBeforeFile={#SourceDir}\README-WINDOWS.txt
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked
; Deliberately unchecked. A user who wants dictation at login can say so;
; a user who does not should not have to notice and undo it.
Name: "startup"; Description: "Запускать при входе в систему"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\SciWhisper.cmd"; IconFilename: "{app}\SciWhisper.ico"
Name: "{group}\Проверка установки"; Filename: "{app}\SciWhisper-Test.cmd"; IconFilename: "{app}\SciWhisper.ico"
Name: "{group}\Настройки"; Filename: "{app}\SciWhisper-Settings.cmd"; IconFilename: "{app}\SciWhisper.ico"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\SciWhisper.cmd"; IconFilename: "{app}\SciWhisper.ico"; Tasks: desktopicon
Name: "{userstartup}\{#AppName}"; Filename: "{app}\SciWhisper.cmd"; IconFilename: "{app}\SciWhisper.ico"; Tasks: startup

[Run]
Filename: "{app}\SciWhisper-Test.cmd"; Description: "Проверить установку (без микрофона и без сети)"; Flags: postinstall shellexec skipifsilent

[UninstallDelete]
; Only what the installer and the updater created. The staging and backup
; directories are siblings of {app} and are left behind by an interrupted
; update; the model pack is unpacked into {app}\whisper by the user.
Type: filesandordirs; Name: "{app}\whisper"
Type: dirifempty; Name: "{app}"

; Nothing below {userappdata} is removed. Settings, the dictionary the user
; edited and anything they kept are theirs, and an uninstaller that deletes
; a user's configuration has decided something that was not its to decide.
