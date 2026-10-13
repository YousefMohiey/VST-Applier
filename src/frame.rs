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

/// Fixed title-bar height (also the drag strip height).
pub const BAR_H: f32 = 52.0;
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

/// 40x30 painted window button with native-feeling hover (red for Close).
pub fn win_button(ui: &mut Ui, kind: WinBtn, maximized: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 34.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() || resp.is_pointer_button_down_on() {
        let fill = match kind {
            WinBtn::Close => Color32::from_rgb(190, 40, 70),
            _ => Color32::from_white_alpha(16),
        };
        p.rect_filled(rect, egui::CornerRadius::same(4), fill);
    }
    let c = rect.center();
    let s = Stroke::new(1.8_f32, Color32::from_rgb(214, 228, 245));
    match kind {
        WinBtn::Min => {
            p.line_segment([c + vec2(-6.0, 0.0), c + vec2(6.0, 0.0)], s);
        }
        WinBtn::MaxRestore => {
            if maximized {
                // Restore: back window offset up-right + front window.
                let b = Rect::from_center_size(c + vec2(2.0, -2.0), vec2(9.0, 9.0));
                p.rect_stroke(b, egui::CornerRadius::same(1), s, StrokeKind::Middle);
                let f = Rect::from_center_size(c + vec2(-2.0, 2.0), vec2(9.0, 9.0));
                p.rect_filled(f, egui::CornerRadius::same(1), Color32::from_rgb(18, 24, 34));
                p.rect_stroke(f, egui::CornerRadius::same(1), s, StrokeKind::Middle);
            } else {
                let r = Rect::from_center_size(c, vec2(11.0, 11.0));
                p.rect_stroke(r, egui::CornerRadius::same(1), s, StrokeKind::Middle);
            }
        }
        WinBtn::Close => {
            p.line_segment([c + vec2(-5.5, -5.5), c + vec2(5.5, 5.5)], s);
            p.line_segment([c + vec2(5.5, -5.5), c + vec2(-5.5, 5.5)], s);
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
        });
}
