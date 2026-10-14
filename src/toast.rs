//! Toast stack (storytold/craft-study `notices.rs` pattern): transient
//! confirmations that don't hijack the status bar. Anchored above the
//! status bar, bottom-right; max 3; 4 s auto-expiry with hover-pause;
//! frameless × dismiss. Expiry wakes exactly on time via
//! `request_repaint_after`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::theme;

const MAX_TOASTS: usize = 3;
const TOAST_SECS: u64 = 4;

struct Toast {
    born: Instant,
    msg: String,
    err: bool,
}

#[derive(Default)]
pub struct Toasts {
    items: VecDeque<Toast>,
}

impl Toasts {
    pub fn push(&mut self, msg: String, err: bool) {
        self.items.push_back(Toast { born: Instant::now(), msg, err });
        while self.items.len() > MAX_TOASTS {
            self.items.pop_front();
        }
    }

    pub fn ok(&mut self, msg: impl Into<String>) {
        self.push(msg.into(), false);
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        crate::log_line(&format!("toast-err: {msg}"));
        self.push(msg, true);
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if self.items.is_empty() {
            return;
        }
        let mut dismiss = None;
        let area = egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-20.0, -46.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for (i, t) in self.items.iter().enumerate() {
                    egui::Frame::default()
                        .fill(theme::CARD)
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            if t.err { theme::CORAL } else { theme::CARD_BORDER },
                        ))
                        .corner_radius(egui::CornerRadius::same(theme::RADIUS_MD))
                        .inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 10 })
                        .outer_margin(egui::Margin { top: 6, ..Default::default() })
                        .shadow(egui::Shadow {
                            offset: [0, 8],
                            blur: 24,
                            spread: 0,
                            color: egui::Color32::from_black_alpha(140),
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // Launcher-style 6x6 status square.
                                let (r, _) = ui.allocate_exact_size(
                                    egui::vec2(6.0, 6.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    egui::Rect::from_center_size(r.center(), egui::vec2(6.0, 6.0)),
                                    egui::CornerRadius::ZERO,
                                    if t.err { theme::CORAL } else { theme::TEAL },
                                );
                                let color = if t.err { theme::CORAL } else { theme::TEXT };
                                ui.label(egui::RichText::new(&t.msg).size(13.0).color(color));
                                if ui
                                    .small_button("×")
                                    .on_hover_text("Dismiss")
                                    .clicked()
                                {
                                    dismiss = Some(i);
                                }
                            });
                        });
                }
            });
        let hovered = area.response.hovered();
        if let Some(i) = dismiss {
            self.items.remove(i);
        }
        if !hovered {
            // Expire old toasts; wake exactly when the next one lapses.
            let mut next_wake = None;
            self.items.retain(|t| {
                let age = t.born.elapsed();
                if age >= Duration::from_secs(TOAST_SECS) {
                    false
                } else {
                    let left = Duration::from_secs(TOAST_SECS) - age;
                    next_wake = Some(next_wake.map_or(left, |n: Duration| n.min(left)));
                    true
                }
            });
            if let Some(w) = next_wake {
                ctx.request_repaint_after(w);
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}
