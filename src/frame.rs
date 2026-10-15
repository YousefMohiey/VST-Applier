//! Borderless window frame: custom title bar + edge resizing.
//!
//! Pure egui/eframe APIs, verified against the installed versions
//! (egui/egui-winit 0.31.1): `ViewportBuilder::with_decorations(false)`
//! removes the OS frame; `StartDrag` moves the window via winit's
//! `drag_window`; `BeginResize(dir)` resizes via winit's
//! `drag_resize_window`; `Minimized`/`Maximized`/`Close` behave like the
//! native buttons. No new dependencies, no HWND hacks — DPI and
//! multi-monitor behavior come from the backend.

use egui::{
    pos2, Color32, CursorIcon, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, vec2,
};
use egui::viewport::ResizeDirection;

/// Fixed title-bar height (also the drag strip height): 4px air above
/// the 34px control row and 4px below it — the row used to sit in a
/// 52px strip, leaving a ~14px dead band under the title.
pub const BAR_H: f32 = 42.0;
/// Edge resize strip thickness.
const EDGE: f32 = 5.0;
/// Corner grip size.
const CORNER: f32 = 14.0;

pub fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

pub fn toggle_maximize(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized(ctx)));
}

/// Title-bar drag background. Call FIRST inside the top panel (after
/// forcing its min height) so later content paints above it and wins
/// clicks; empty bar areas start a window drag, double-click toggles
/// maximize. Buttons/popups added afterwards are unaffected.
pub fn title_drag_bg(ui: &mut Ui) {
    ui.set_min_height(BAR_H);
    let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), BAR_H));
    let resp = ui.interact(rect, Id::new("title_drag"), Sense::click_and_drag());
    if resp.drag_started() {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        toggle_maximize(ui.ctx());
    }
}

#[derive(Clone, Copy)]
pub enum WinBtn {
    Min,
    MaxRestore,
    Close,
}

/// Square caption button in craft-launcher style: hairline separators,
/// hover fill = hairline tone with strong ink, Close turns red. Glyphs
/// are drawn as 1px 10px-wide line art like the launcher's.
pub fn win_button(ui: &mut Ui, kind: WinBtn, maximized: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(46.0, 34.0), Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered() || resp.is_pointer_button_down_on();
    let (fill, ink) = match (kind, hovered) {
        (WinBtn::Close, true) => (Color32::from_rgb(0xc4, 0x2b, 0x1c), Color32::WHITE),
        (_, true) => (crate::theme::line(), Color32::WHITE),
        (_, false) => (Color32::TRANSPARENT, crate::theme::muted()),
    };
    p.rect_filled(rect, egui::CornerRadius::ZERO, fill);
    // Hairline on the left of each cell, like their header cells.
    p.vline(
        rect.left() + 0.5,
        rect.y_range(),
        Stroke::new(1.0_f32, crate::theme::line()),
    );
    let c = rect.center();
    let s = Stroke::new(1.0_f32, ink);
    let h = 5.0;
    match kind {
        WinBtn::Min => {
            p.hline(c.x - h..=c.x + h, c.y, s);
        }
        WinBtn::MaxRestore => {
            if maximized {
                // A window in front of another.
                let front = Rect::from_min_size(c + vec2(-h, -h + 2.0), vec2(2.0 * h - 2.0, 2.0 * h - 2.0));
                p.rect_stroke(front, egui::CornerRadius::ZERO, s, StrokeKind::Inside);
                p.hline(c.x - h + 2.0..=c.x + h, c.y - h, s);
                p.vline(c.x + h, c.y - h..=c.y + h - 2.0, s);
            } else {
                p.rect_stroke(
                    Rect::from_center_size(c, vec2(2.0 * h, 2.0 * h)),
                    egui::CornerRadius::ZERO,
                    s,
                    StrokeKind::Inside,
                );
            }
        }
        WinBtn::Close => {
            p.line_segment([c + vec2(-h, -h), c + vec2(h, h)], s);
            p.line_segment([c + vec2(-h, h), c + vec2(h, -h)], s);
        }
    }
    let resp = resp.on_hover_text(match kind {
        WinBtn::Min => "Minimize",
        WinBtn::MaxRestore => "Maximize / restore",
        WinBtn::Close => "Close",
    });
    resp
}

/// Invisible edge/corner resize grips above all panels (foreground Area).
/// Skipped while maximized. The 5px strips overlap panel edges — scrollbars
/// stay grabbable just inside them.
pub fn edge_zones(ctx: &egui::Context) {
    if is_maximized(ctx) {
        return;
    }
    egui::Area::new(Id::new("frame_edges"))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let r = ctx.screen_rect();
            let m = EDGE;
            let c = CORNER;
            // (zone rect, resize dir, cursor)
            let zones: [(Rect, ResizeDirection, CursorIcon); 8] = [
                (
                    Rect::from_min_max(pos2(r.min.x, r.min.y), pos2(r.min.x + m, r.max.y)),
                    ResizeDirection::West,
                    CursorIcon::ResizeHorizontal,
                ),
                (
                    Rect::from_min_max(pos2(r.max.x - m, r.min.y), pos2(r.max.x, r.max.y)),
                    ResizeDirection::East,
                    CursorIcon::ResizeHorizontal,
                ),
                (
                    Rect::from_min_max(pos2(r.min.x, r.min.y), pos2(r.max.x, r.min.y + m)),
                    ResizeDirection::North,
                    CursorIcon::ResizeVertical,
                ),
                (
                    Rect::from_min_max(pos2(r.min.x, r.max.y - m), pos2(r.max.x, r.max.y)),
                    ResizeDirection::South,
                    CursorIcon::ResizeVertical,
                ),
                (
                    Rect::from_min_max(pos2(r.min.x, r.min.y), pos2(r.min.x + c, r.min.y + c)),
                    ResizeDirection::NorthWest,
                    CursorIcon::ResizeNwSe,
                ),
                (
                    Rect::from_min_max(pos2(r.max.x - c, r.min.y), pos2(r.max.x, r.min.y + c)),
                    ResizeDirection::NorthEast,
                    CursorIcon::ResizeNeSw,
                ),
                (
                    Rect::from_min_max(pos2(r.min.x, r.max.y - c), pos2(r.min.x + c, r.max.y)),
                    ResizeDirection::SouthWest,
                    CursorIcon::ResizeNeSw,
                ),
                (
                    Rect::from_min_max(pos2(r.max.x - c, r.max.y - c), pos2(r.max.x, r.max.y)),
                    ResizeDirection::SouthEast,
                    CursorIcon::ResizeNwSe,
                ),
            ];
            for (k, (rect, dir, cursor)) in zones.into_iter().enumerate() {
                let resp = ui.interact(rect, Id::new(("edge", k)), Sense::drag());
                if resp.hovered() {
                    ctx.set_cursor_icon(cursor);
                }
                if resp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
                }
            }
            // Window border hairline (their `window_edges`): one strong
            // 1px rule around the whole frame.
            ui.painter().rect_stroke(
                r,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0_f32, crate::theme::line_strong()),
                StrokeKind::Inside,
            );
        });
}
