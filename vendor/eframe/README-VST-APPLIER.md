# Vendored eframe 0.31.1 — VST-Applier patch

This is an unmodified copy of `eframe-0.31.1` from crates.io **except for
one hunk** in `src/native/run.rs` (`WinitAppWrapper::check_redraw_requests`).

## The bug

When a repaint is due, eframe sets `ControlFlow::Poll` and calls
`Window::request_redraw()`. On Windows, `request_redraw` goes through
`RedrawWindow(RDW_INTERNALPAINT)`, and a **hidden** window
(`SW_HIDE` / `ViewportCommand::Visible(false)`) never actually receives
`WM_PAINT`. So the `RedrawRequested` event that would run `update()`
and reset the control flow back to `ControlFlow::Wait` never arrives,
and the winit message pump busy-spins at ~100% of one core.

Measured on the real app (VST-Applier trace): visible idle ~1-10% of a
core -> close-to-tray (hidden) ~93-99% of one core, indefinitely.
Minimized-but-visible windows keep their paints and stay at ~1%.

## The patch

In `check_redraw_requests`, when a due repaint's window is invisible,
the request is dropped (`retain` returns false) and no redraw is
requested:

```rust
if let Some(window) = self.winit_app.window(*window_id) {
    if window.is_visible() == Some(false) {
        log::trace!("dropping redraw request for hidden {window_id:?}");
        return false;
    }
}
```

`ControlFlow` stays `Wait`/`WaitUntil`, the loop parks, and CPU drops to
~0 while hidden. When the window is shown again the OS invalidates it
and delivers a fresh `WM_PAINT`, which is exactly the OS-level reveal
path (`ShowWindow` in `src/win.rs`) the app already relies on for tray
clicks and second-instance wake-ups.

Keep this patch pointed at the vendor dir until (if ever) the same
handling lands upstream; re-check after every eframe bump.
