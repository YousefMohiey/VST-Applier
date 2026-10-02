# TZ 02: Plugin Chain Reorder With Up/Down And Drag-Drop

## Goal

Allow users to change the order of plugins in the chain without removing and re-adding them. Order matters because the audio passes through plugins sequentially.

## Scope

- C# WinForms UI/controller layer.
- Prefer changes in `SnjVoiceChanger/Form1.cs` and `SnjVoiceChanger/Form1.Designer.cs`.
- Reuse existing `VstPluginChainItem` instances; do not recreate native plugin hosts during reorder.
- Do not touch native VST host code.
- Do not change `AudioRoutingService` processing order beyond the existing `GetPluginChainSnapshot()` order.

## Research Notes

- WinForms `ListView` supports full-row selection, columns, checked items, and item collections suitable for a small plugin chain UI. Microsoft documents that `ListView` can display check boxes through `CheckBoxes` and can use `FullRowSelect` in details view.
- WinForms drag/drop uses `DoDragDrop`, `DragOver`, and `DragDrop`; Microsoft documents that `DragOver` fires while the pointer moves over a possible drop target and that `DoDragDrop` resolves the target and effect.

Sources:

- Microsoft Learn: `ListView` supports check boxes, item collections, columns, and full-row selection: https://learn.microsoft.com/en-us/dotnet/api/system.windows.forms.listview
- Microsoft Learn: `Control.DragOver` / drag-drop event behavior: https://learn.microsoft.com/en-us/dotnet/api/system.windows.forms.control.dragover

## Required Behavior

- Add `Up` and `Down` buttons near existing `Remove` and `Editor`.
- `Up` moves selected plugin one slot earlier.
- `Down` moves selected plugin one slot later.
- Buttons are disabled when no plugin is selected.
- `Up` is disabled for the first selected plugin.
- `Down` is disabled for the last selected plugin.
- Reorder must preserve the exact same `VstPluginChainItem` object, including native host and editor state.
- If route is running, reordering should use TZ 01 auto-restart behavior.

## Drag-Drop Behavior

Preferred:

- Support dragging a row within the chain control to reorder.
- Use single-row drag only.
- Drop before target row when dropping on a row.
- Drop at end when dropping below the last visible item.
- Preserve selected row after move.
- If route is running, drag-drop reorder triggers the same auto-restart as Up/Down.

Fallback:

- If drag/drop becomes risky, implement Up/Down first and leave drag/drop for a later pass.

## Proposed UI Direction

Replace `pluginChainListBox` with a `ListView` if needed for checkbox work in TZ 03:

- `View = View.Details`
- `CheckBoxes = true`
- `FullRowSelect = true`
- `HideSelection = false`
- one main column named `Plugin`
- `ListViewItem.Tag = VstPluginChainItem`

If TZ 03 is not implemented in the same pass, Up/Down can work against the current `ListBox`, but the final preferred control is `ListView`.

## Acceptance Criteria

- User can reorder plugins with `Up` and `Down`.
- Reorder changes `GetPluginChainSnapshot()` order.
- Existing plugin settings are preserved after reorder.
- Route auto-restarts if it was running.
- Drag/drop works if implemented; if not, Up/Down still works reliably.

## Risk Notes

- Do not dispose plugin hosts during reorder.
- Avoid replacing objects with newly loaded plugins.
- Drag/drop must not accidentally start external file drop handling.

