//! System-tray presence (mirrors the C# `NotifyIcon` behavior).
//!
//! Always visible while running. Menu: Open / Exit. Window-close hides to
//! the tray when `CloseToTray` is set (handled in `App::update`).

use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

pub enum TrayAction {
    None,
    Open,
    Quit,
}

pub struct Tray {
    _icon: Option<TrayIcon>,
    open_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
}

fn app_icon() -> Option<tray_icon::Icon> {
    // Redesigned mic artwork (assets/icon-32.rgba), baked in at build
    // time so no image decoder is needed at runtime.
    let rgba = include_bytes!("../assets/icon-32.rgba").to_vec();
    tray_icon::Icon::from_rgba(rgba, 32, 32).ok()
}

impl Tray {
    pub fn new() -> Self {
        let mut open_id = tray_icon::menu::MenuId::new("open");
        let mut quit_id = tray_icon::menu::MenuId::new("quit");
        let icon = (|| {
            let menu = Menu::new();
            let open = MenuItem::new("Open VST-Applier", true, None);
            let quit = MenuItem::new("Exit", true, None);
            open_id = open.id().clone();
            quit_id = quit.id().clone();
            menu.append(&open).ok()?;
            menu.append(&quit).ok()?;
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip("VST-Applier")
                .with_icon(app_icon()?)
                .build()
                .ok()
        })();
        Tray { _icon: icon, open_id, quit_id }
    }

    pub fn poll(&self) -> TrayAction {
        if self._icon.is_none() {
            return TrayAction::None;
        }
        // Left-click opens the window directly (never a menu).
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                // TEMP-PROBE tray events.
                crate::log_line("TRAY-EVENT left-click");
                return TrayAction::Open;
            }
        }
        let mut action = TrayAction::None;
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            // TEMP-PROBE tray events.
            crate::log_line(&format!("TRAY-EVENT id={:?}", ev.id));
            if ev.id == self.open_id {
                action = TrayAction::Open;
            } else if ev.id == self.quit_id {
                return TrayAction::Quit;
            }
        }
        action
    }
}
