; VST-Applier installer — Inno Setup 6.
; Build:  "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" VstApplierRs.iss
; Stage first: installer\app must hold
;   vst-applier.exe, Vst2HostNative.dll, Vst3HostNative.dll (release build),
;   Plugins\  (bundled VSTs) and common\ (VB-CABLE driver pack).
; Same AppId as v0.2.0 -> in-place upgrade of an existing install.
#define AppName "VST-Applier"
#define AppVersion "0.4.7"
#define AppPublisher "Yousef Mohiey"
#define AppExeName "vst-applier.exe"
#define PublishDir "app"

[Setup]
AppId={{678020FF-B838-47D6-AA21-08FA5FE150A8}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppUpdatesURL=https://github.com/YousefMohiey/VST-Applier/releases/latest
DefaultDirName={autopf}\VST-Applier
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputDir=.
OutputBaseFilename=VST-Applier-Setup-v0.4.7
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#AppExeName}
SetupIconFile=..\assets\app.ico
WizardStyle=modern
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoProductVersion={#AppVersion}
VersionInfoDescription={#AppName} {#AppVersion} Setup
VersionInfoCompany={#AppPublisher}
CloseApplications=yes
RestartApplications=no
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: checkedonce
Name: "vbcable"; Description: "Install the VB-CABLE virtual audio driver (recommended; asks for admin + needs one reboot)"; GroupDescription: "Audio routing:"; Flags: checkedonce

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
Filename: "{app}\{#AppExeName}"; Description: "Launch {#AppName}"; Flags: postinstall nowait skipifsilent

[Code]
{ VB-CABLE presence: the driver's service key (its name varies between
  packs — 'VB-Cable' on current ones), VB-Audio's registry root, or the
  Win10 driver file. Any of them counts as already installed. }
function VBCableInstalled(): Boolean;
begin
  Result :=
    RegKeyExists(HKLM, 'SYSTEM\CurrentControlSet\Services\VB-Cable') or
    RegKeyExists(HKLM, 'SYSTEM\CurrentControlSet\Services\VBAudioVBCable') or
    RegKeyExists(HKLM64, 'SOFTWARE\VB-Audio\VBCABLE') or
    FileExists(ExpandConstant('{sys}\drivers\vbaudio_cable64_win10.sys'));
end;

{ Run the VB-CABLE installer ELEVATED. As a plain child of setup it
  started without admin rights, so its own "Install Driver" button
  failed with an admin error. ShellExec('runas', …) asks for consent
  first and the driver installer then runs with the rights it needs. }
procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
  VBCableExe: String;
begin
  if (CurStep = ssPostInstall) and (not WizardSilent) and
     WizardIsTaskSelected('vbcable') and (not VBCableInstalled()) then
  begin
    VBCableExe := ExpandConstant('{app}\common\VBCABLE_Setup_x64.exe');
    if not ShellExec('runas', VBCableExe, '', ExtractFileDir(VBCableExe),
                     SW_SHOWNORMAL, ewWaitUntilTerminated, ResultCode) then
      MsgBox('VB-CABLE setup could not start (admin approval was declined?).'
             + #13#10 +
             'You can install it later by right-clicking "' +
             VBCableExe + '" and choosing "Run as administrator".',
             mbInformation, MB_OK);
  end;
end;
