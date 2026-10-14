; VST-Applier (Rust) installer — mirrors publish/VstApplier.iss from the
; C# app: same install dir, same VB-CABLE postinstall option, same
; desktop-shortcut task. AppId is new (side-by-side uninstall entry).
#define AppName "VST-Applier"
#define AppVersion "0.2.0"
#define AppPublisher "Yousef Mohiey"
#define AppExeName "vst-applier.exe"
#define PublishDir "app"

[Setup]
AppId={{678020FF-B838-47D6-AA21-08FA5FE150A8}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\VST-Applier
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputDir=.
OutputBaseFilename=VST-Applier-Setup-v0.2.0
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#AppExeName}
SetupIconFile=..\assets\app.ico
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: checkedonce

[Files]
; App + native host DLLs.
Source: "{#PublishDir}\vst-applier.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PublishDir}\Vst2HostNative.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PublishDir}\Vst3HostNative.dll"; DestDir: "{app}"; Flags: ignoreversion
; Bundled noise/gate plugins (ReaPlugs, RNNoise, DeepFilterNet, ...).
Source: "{#PublishDir}\Plugins\*"; DestDir: "{app}\Plugins"; Flags: recursesubdirs ignoreversion
; VB-CABLE driver pack (optional postinstall step below).
Source: "{#PublishDir}\common\*"; DestDir: "{app}\common"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{commondesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\common\VBCABLE_Driver_Pack45\VBCABLE_Setup_x64.exe"; Description: "Install VB-CABLE virtual audio driver (needs one reboot)"; Flags: postinstall shellexec skipifsilent
Filename: "{app}\{#AppExeName}"; Description: "Launch {#AppName}"; Flags: postinstall nowait skipifsilent
