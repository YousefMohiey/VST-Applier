# TZ 04: Plugin Chain Checkbox Click Scope

Context:
- Snj Voice Changer is a C# WinForms app.
- The user explicitly forbids running builds/tests; do not run any build or test commands.
- Do not touch audio routing, VST native host, driver branch/files, third_party files, installer files, or core processing.
- There may be unrelated worktree changes; do not revert them.

Task:
Fix the plugin chain list interaction so checkbox toggling does not conflict with row double-click editor opening.

Current issue:
- `pluginChainListBox` is a `CheckedListBox` with `CheckOnClick = true`.
- A double-click on a row is interpreted as two single clicks, toggling the plugin enable checkbox.
- Desired behavior:
  - Single click on the checkbox glyph toggles plugin enable/bypass.
  - Single click on the row text/empty row area only selects the row.
  - Double-click on the row text/empty row area opens the plugin editor, preserving current behavior.
  - Double-click on the checkbox glyph should not open the editor; it should be treated as checkbox interaction only.

Scope:
- Prefer changes in `SnjVoiceChanger/Form1.cs` and `SnjVoiceChanger/Form1.Designer.cs` only.
- Keep `VstPluginChainItem.IsEnabled` behavior and the existing auto-restart logic intact.
- Do not change plugin processing or native host code.

Implementation hint:
- Set `pluginChainListBox.CheckOnClick = false`.
- Add mouse handling that detects whether the click is inside the checkbox glyph area for the clicked row.
- Toggle check state only for checkbox-glyph clicks, ideally by calling `SetItemChecked` so the existing `ItemCheck` handler updates `IsEnabled` and restarts the route when needed.
- Keep row double-click editor behavior, but guard it so it ignores double-clicks on the checkbox glyph.

Acceptance:
- Clicking a row selects it and does not toggle enabled/bypassed.
- Clicking the checkbox toggles enabled/bypassed.
- Double-clicking a row opens the editor and does not toggle the checkbox.
- Double-clicking checkbox area does not open the editor.
- No build/test commands were run.
