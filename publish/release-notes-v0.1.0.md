VST-Applier v0.1.0 - first release.

Windows voice changer: routes your microphone through a mixed VST2/VST3 plugin chain into a virtual audio cable (VB-CABLE).

Features:

- Named setup profiles: Create, Save and Delete full setups (devices, buffer size, plugin folder, plugin chain). Save writes to the equipped profile; Create suggests the first unused "New profile" name.
- Automatic session restore: your whole setup comes back on launch, including each plugin's parameter state.
- Plugin state save/load for both VST2 (chunk or parameter dump) and VST3 (component/controller state or parameter dump).
- Single instance: launching the app again (desktop shortcut, Start menu) brings the running window to the front instead of starting a second copy.
- Close to system tray: closing the window keeps the app running; tray menu has Open and Exit.
- Settings dialog: auto-start microphone routing on launch, close-to-tray toggle, start with Windows.
- Dark UI (deep teal / mint palette) with the project's own icon artwork.

Assets:

- VstApplier_v0.1.0.exe - installer (Program Files, shortcuts, optional VB-CABLE driver install step).
- VstApplier_v0.1.0_win-x64.zip - portable build: extract anywhere and run VstApplier.exe.
