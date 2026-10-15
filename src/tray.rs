//! System-tray presence (mirrors the C# `NotifyIcon` behavior).
//!
//! Always visible while running. Left-click and menu "Open" reveal the
//! window via `win::reveal_main_window` — an OS-level ShowWindow that
//! works even when the eframe event loop is parked on a hidden window.
//! Menu "Exit" reveals (to wake the loop) + a watchdog guarantees the
//! process leaves. Window-close hides to the tray when `CloseToTray` is
//! set (handled in `App::update`).

use std::sync::mpsc;

use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    None,
    Open,
    Quit,
}

pub struct Tray {
    _icon: Option<TrayIcon>,
    rx: mpsc::Receiver<TrayAction>,
}

fn app_icon() -> Option<tray_icon::Icon> {
    // Redesigned mic artwork (assets/icon-32.rgba), baked in at build
    // time so no image decoder is needed at runtime.
    let rgba = include_bytes!("../assets/icon-32.rgba").to_vec();
    tray_icon::Icon::from_rgba(rgba, 32, 32).ok()
}

impl Tray {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let icon = (|| {
            let menu = Menu::new();
            let open = MenuItem::new("Open VST-Applier", true, None);
            let quit = MenuItem::new("Exit", true, None);
            let open_id = open.id().clone();
            let quit_id = quit.id().clone();
            menu.append(&open).ok()?;
            menu.append(&quit).ok()?;
            // Menu picks: queue the action + wake the (possibly dormant)
            // event loop. NOTE: setting a handler redirects events away
            // from `MenuEvent::receiver()` — `poll()` below reads OUR
            // channel instead.
            let tx_menu = tx.clone();
            let ctx_menu = ctx.clone();
            MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
                crate::log_line(&format!("TRAY-EVENT id={:?}", ev.id));
                let action = if ev.id == open_id {
                    TrayAction::Open
                } else if ev.id == quit_id {
                    TrayAction::Quit
                } else {
                    return;
                };
                // OS-level reveal: the deterministic wake while the
                // window is hidden (repaint requests are not reliably
                // delivered to invisible windows on Windows).
                crate::win::reveal_main_window();
                if action == TrayAction::Quit {
                    // Watchdog: a tray Exit must ALWAYS exit, even if the
                    // clean path (stop route + save session) somehow
                    // can't run. The process normally exits first.
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_millis(2500));
                        crate::log_line("quit watchdog: forcing exit");
                        std::process::exit(0);
                    });
                }
                let _ = tx_menu.send(action);
                ctx_menu.request_repaint();
            }));
            // Left-click opens directly (never a menu).
            let tx_click = tx.clone();
            let ctx_click = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = ev
                {
                    crate::log_line("TRAY-EVENT left-click");
                    crate::win::reveal_main_window();
                    let _ = tx_click.send(TrayAction::Open);
                    ctx_click.request_repaint();
                }
            }));
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip("VST-Applier")
                .with_icon(app_icon()?)
                .build()
                .ok()
        })();
        Tray { _icon: icon, rx }
    }

    /// Drain queued actions (non-blocking). The handlers above already
    /// poked the loop awake, so this runs on a live frame.
    pub fn poll(&self) -> TrayAction {
        if self._icon.is_none() {
            return TrayAction::None;
        }
        let mut action = TrayAction::None;
        while let Ok(a) = self.rx.try_recv() {
            // Quit wins immediately; otherwise last action wins.
            if a == TrayAction::Quit {
                return TrayAction::Quit;
            }
            action = a;
        }
        action
    }
}
