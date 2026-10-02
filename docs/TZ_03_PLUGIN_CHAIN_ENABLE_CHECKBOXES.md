# TZ 03: Plugin Enable/Disable Checkboxes

## Goal

Add Reaper-like checkboxes to plugins in the chain. Unchecked plugins stay loaded and keep their editor/settings, but are skipped during audio processing.

## Scope

- C# WinForms UI/controller/model layer.
- Prefer changes in:
  - `SnjVoiceChanger/VstPluginChainItem.cs`
  - `SnjVoiceChanger/Form1.cs`
  - `SnjVoiceChanger/Form1.Designer.cs`
- `AudioRoutingService` can receive only enabled plugins via `GetPluginChainSnapshot()`. Avoid changing audio processing internals.
- Do not touch native VST host code.

## Research Notes

- Microsoft documents `CheckedListBox` as a list with a checkbox beside each item, but it is still a `ListBox` and is less flexible for row-style chain UI.
- Microsoft documents `ListView.CheckBoxes` and `ListView.CheckedItems`; `ListView` also supports full-row selection and details view, which better matches a DAW-style FX chain.

Sources:

- Microsoft Learn: `CheckedListBox` displays a checkbox to the left of each item: https://learn.microsoft.com/en-us/dotnet/api/system.windows.forms.checkedlistbox
- Microsoft Learn: `ListView` supports checkboxes and checked item collections: https://learn.microsoft.com/en-us/dotnet/api/system.windows.forms.listview

## Required Behavior

- Every plugin row in the chain has a checkbox.
- New plugins are checked/enabled by default.
- Unchecking a plugin disables it in processing without removing it.
- Re-checking enables the same plugin instance again.
- Plugin editor/settings/native host must stay alive while disabled.
- Disabled plugins still appear in the chain and can be reordered, edited, or removed.
- If route is running, toggling a checkbox should use TZ 01 auto-restart behavior.

## Data Model

Add enabled state to `VstPluginChainItem`:

```csharp
public bool IsEnabled { get; set; } = true;
```

`ToString()` may include a visual prefix only if still using `ListBox`. If moving to `ListView`, keep `ToString()` simple and show state through the row checkbox.

## Processing Snapshot

Change `GetPluginChainSnapshot()` so it returns only enabled plugins:

```csharp
if (chainItem.IsEnabled)
{
    pluginChain.Add(chainItem);
}
```

Important: disabled plugins must not be disposed and must not be reloaded.

## UI Direction

Preferred:

- Replace current `ListBox` with `ListView`.
- Store `VstPluginChainItem` in `ListViewItem.Tag`.
- Use `ItemChecked` to update `chainItem.IsEnabled`.
- Use a guard flag while programmatically rebuilding rows to avoid accidental restarts.

Fallback:

- Use `CheckedListBox` only if `ListView` integration gets too noisy.

## Acceptance Criteria

- Add plugin: row appears checked.
- Uncheck row: route restarts automatically if running and audio bypasses that plugin.
- Re-check row: same plugin instance returns to processing.
- Open editor before/after disabling: settings are retained.
- Remove still disposes host.
- Reorder still preserves enabled state.

## Risk Notes

- Do not dispose or recreate disabled plugins.
- Do not call native host bypass APIs; none exist yet.
- Avoid event storms: `ItemChecked` fires during programmatic changes unless guarded.

