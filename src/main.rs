// GUI subsystem in release only (photocraft pattern): double-click
// launches never flash a console, while debug keeps stdout for probes.
// (Probes like --sessions still print when run from an existing console:
// no-new-console only means "don't create one".)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// NOTE: release builds are GUI-subsystem (see crate attr above), so CLI
// probes only print when run from an existing console.

mod autostart;
mod conflict;
mod editor;
mod frame;
mod guard;
mod icons;
mod theme;
mod toast;
mod tray;
mod update;
mod win;

use std::cell::Cell;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use audio_engine::{input_devices, output_devices, start, ChainPlugin, RunningRoute, ALLOWED_BLOCKS};
use eframe::egui;

use app_state::{
    delete_profile, is_valid_profile_name, load_profile, load_session, load_settings,
    profile_names, save_profile, save_session, save_settings, scan, AppSettings, VoiceSetupProfile,
    default_plugin_folder,
};
use vst_host_sys::{decode_state_field, encode_state_field, PluginFormat};

/// eframe state file (window geometry + egui memory). Corrupt values
/// (NaN/inf, from a bad write or GPU hiccup) abort startup deep inside
/// DPI code — photocraft lesson: delete-on-corrupt, never repair.
fn ui_state_path() -> std::path::PathBuf {
    app_state::settings_dir().join("ui_state")
}

fn sanitize_ui_state(path: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    if text.contains("NaN") || text.contains("inf") {
        let _ = std::fs::remove_file(path);
        log_line("ui_state: dropped corrupt geometry file");
    }
}

/// Window + taskbar icon: the redesigned mic artwork, baked in as raw
/// RGBA so no image decoder is needed at runtime.
fn window_icon() -> std::sync::Arc<egui::IconData> {
    std::sync::Arc::new(egui::IconData {
        rgba: include_bytes!("../assets/icon-256.rgba").to_vec(),
        width: 256,
        height: 256,
    })
}

fn main() {
    // --tray: start hidden in the tray (used by the autostart entry).
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--sessions") {
        println!("old app pids: {:?}", conflict::old_app_pids());
        guard::dump_sessions();
        return;
    }
    if args.iter().any(|a| a == "--quit-old") {
        let n = conflict::quit_old_app();
        println!("asked {n} old-app window(s) to close");
        return;
    }
    if args.iter().any(|a| a == "--unmute-old") {
        let n = guard::unmute_exe("VstApplier.exe", None);
        println!("unmuted {n} old-app session(s)");
        return;
    }
    // Headless self-update probes (scripting + verification without UI):
    // --check-update prints the verdict, --self-update applies it.
    if args.iter().any(|a| a == "--check-update") {
        match update::check_latest() {
            Ok(None) => println!("current (v{})", update::APP_VERSION),
            Ok(Some(r)) => println!("available: {} {} {}", r.tag, r.name, r.zip_url),
            Err(e) => println!("check failed: {e}"),
        }
        return;
    }
    if args.iter().any(|a| a == "--self-update") {
        match update::check_latest() {
            Ok(None) => println!("current (v{})", update::APP_VERSION),
            Ok(Some(rel)) => {
                println!("downloading {} …", rel.tag);
                match update::download_and_stage(&rel) {
                    Ok(stage) => {
                        println!("installing + restarting …");
                        let msg = update::install_and_restart(&stage);
                        println!("{msg}");
                    }
                    Err(e) => println!("download failed: {e}"),
                }
            }
            Err(e) => println!("check failed: {e}"),
        }
        return;
    }
    let start_hidden = args.iter().any(|a| a == "--tray");

    // Single instance: a second launch signals the first and exits.
    let show_rx = single_instance_guard();
    if show_rx.is_none() {
        eprintln!("second instance: signaled the running window, exiting");
        return; // Another instance is running; it will come to front.
    }

    let state_path = ui_state_path();
    sanitize_ui_state(&state_path);

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VST-Applier")
            .with_inner_size(initial_size())
            .with_min_inner_size([1024.0, 700.0])
            .with_decorations(false)
            .with_icon(window_icon())
            .with_visible(!start_hidden),
        // Remember window geometry across launches (photocraft pattern);
        // center only on first run, otherwise eframe restores position.
        persistence_path: Some(state_path),
        centered: !ui_state_path().exists(),
        // Grow a restored strip (e.g. minimized 160x64) back to minimum.
        window_builder: Some(Box::new(|b| {
            const MIN: egui::Vec2 = egui::Vec2 { x: 1024.0, y: 700.0 };
            match (b.inner_size, b.min_inner_size) {
                (Some(size), Some(min)) => {
                    let grown = egui::Vec2 {
                        x: size.x.max(min.x).max(MIN.x),
                        y: size.y.max(min.y).max(MIN.y),
                    };
                    let mut b = b;
                    if grown != size {
                        let d = (grown - size) * 0.5;
                        b.position = b.position.map(|p| egui::Pos2 { x: p.x - d.x, y: p.y - d.y });
                    }
                    b.inner_size = Some(grown);
                    b
                }
                _ => b,
            }
        })),
        ..Default::default()
    };
    let result = eframe::run_native(
        "VST-Applier",
        opts,
        Box::new(|cc| {
            cc.egui_ctx.set_style(app_style());
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(App::new(cc, show_rx.unwrap(), start_hidden)))
        }),
    );
    if let Err(e) = &result {
        eprintln!("run_native failed: {e:?}");
    }
}

/// Full style: craft-launcher visuals + type scale and spacing
/// (`text_styles`/`spacing` live on `Style`, not `Visuals`).
fn app_style() -> egui::Style {
    theme::style()
}

/// Card container: flat surface + hairline (craft-launcher discipline —
/// cards carry no shadow and no rounding), red section icon, mono
/// upper-case title + faint subtitle, optional header-right widgets.
/// Never size content from
/// `available_width()` here (early-frame full-width reads ratchet the
/// side panel open permanently — side width is capped instead).
fn card2(
    ui: &mut egui::Ui,
    ic: icons::Icon,
    title: &str,
    subtitle: Option<&str>,
    right: Option<impl FnOnce(&mut egui::Ui)>,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::default()
        .fill(theme::BG)
        .stroke(egui::Stroke::new(1.0_f32, theme::LINE))
        .corner_radius(egui::CornerRadius::ZERO)
        .inner_margin(egui::Margin::same(14))
        .outer_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 6,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                icons::icon(ui, ic, theme::CORAL);
                ui.vertical(|ui| {
                    let galley = {
                        let format = egui::TextFormat {
                            font_id: egui::FontId::new(12.0, egui::FontFamily::Monospace),
                            color: theme::INK_STRONG,
                            extra_letter_spacing: 1.4,
                            ..Default::default()
                        };
                        ui.painter().layout_job(egui::text::LayoutJob::single_section(
                            title.to_uppercase(),
                            format,
                        ))
                    };
                    ui.label(galley);
                    if let Some(sub) = subtitle {
                        ui.label(
                            egui::RichText::new(sub)
                                .size(11.5)
                                .color(theme::TEXT_FAINT),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(r) = right {
                        r(ui);
                    }
                });
            });
            ui.add_space(6.0);
            add(ui);
        });
}
/// Usable primary-monitor work area (taskbar excluded).
fn work_area() -> (i32, i32) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETWORKAREA};
    unsafe {
        let mut r = RECT::default();
        if SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut r as *mut _ as *mut core::ffi::c_void),
            Default::default(),
        )
        .is_ok()
        {
            return ((r.right - r.left).max(1024), (r.bottom - r.top).max(720));
        }
    }
    (1536, 1024)
}

/// Initial window size from the work area: roomy on 1440p, compact on
/// 1080p, never larger than the reference 1536x1024.
fn initial_size() -> [f32; 2] {
    let (w, h) = work_area();
    [
        (w as f32 * 0.72).clamp(1140.0, 1470.0),
        (h as f32 * 0.72).clamp(700.0, 840.0),
    ]
}

/// Returns the show-window channel for the first instance, or None when a
/// previous instance already holds the mutex (after signaling it).
fn single_instance_guard() -> Option<mpsc::Receiver<()>> {
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, SetEvent, EVENT_ALL_ACCESS, EVENT_MODIFY_STATE,
    };
    unsafe {
        // Atomic first/second decision via the mutex itself: creating an
        // existing named mutex reports ERROR_ALREADY_EXISTS, so two
        // processes launched in the same instant can never both believe
        // they are first (the old OpenEvent-then-CreateEvent gap allowed
        // exactly that, leaving twin instances + twin tray icons).
        let mutex = CreateMutexW(None, true, w!("VstApplierRs.SingleInstance")).ok();
        let already = GetLastError() == ERROR_ALREADY_EXISTS;
        if already {
            // Second instance: the first may still be creating its event;
            // retry briefly, then wake it and exit either way.
            for _ in 0..30 {
                if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, w!("VstApplierRs.ShowWindow")) {
                    let _ = SetEvent(ev);
                    let _ = CloseHandle(ev);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            return None;
        }
        // First instance: publish the wake event, keep the mutex for life.
        // NOTE: HANDLE is Copy, so mem::forget would be a no-op; Box::leak
        // keeps the handle open until process exit instead.
        Box::leak(Box::new(mutex));
        let _event = CreateEventW(None, false, false, w!("VstApplierRs.ShowWindow")).ok()?;
        let (tx, rx) = mpsc::channel();
        // Open the event handle inside the thread: HANDLE is not Send.
        std::thread::spawn(move || {
            use windows::Win32::Foundation::WAIT_OBJECT_0;
            use windows::Win32::System::Threading::WaitForSingleObject;
            // ALL_ACCESS: waiting needs SYNCHRONIZE, which MODIFY_STATE
            // alone does not grant (the wait would fail forever and the
            // first window would never wake on re-launch).
            let Ok(ev) = OpenEventW(EVENT_ALL_ACCESS, false, w!("VstApplierRs.ShowWindow")) else {
                return;
            };
            loop {
                if WaitForSingleObject(ev, 500) == WAIT_OBJECT_0 {
                    log_line("second-instance wake received");
                    // OS-level reveal first: guaranteed even when the
                    // window is hidden and the event loop is parked, so
                    // the relaunch always surfaces the running app.
                    win::reveal_main_window();
                    if tx.send(()).is_err() {
                        break;
                    }
                }
            }
        });
        Some(rx)
    }
}

#[derive(Debug, Clone)]
struct ChainItem {
    name: String,
    path: std::path::PathBuf,
    format: PluginFormat,
    enabled: bool,
    /// Last attach error (`None` = attached OK or not attempted yet).
    error: Option<String>,
    /// Host currently attached (route running). Drives the status dot.
    attached: bool,
}

/// Snapshot undo for chain edits (figcraft pattern: cheap snapshots of a
/// small model beat inverse operations). States ride along, so undo
/// restores knob positions too. Toggle-enable is excluded — one click
/// reverses it already.
#[derive(Debug, Clone, Default)]
struct ChainSnap {
    chain: Vec<ChainItem>,
    pending: Vec<Option<String>>,
}

struct App {
    inputs: Vec<(String, String)>,
    outputs: Vec<(String, String)>,
    sel_input: String,
    sel_output: String,
    sel_block: u32,
    route: Option<RunningRoute>,
    status: String,
    /// Full diagnostic line for the status bar tooltip.
    status_diag: String,
    /// Background plugin scan in flight (keeps the UI responsive).
    scan_rx: Option<mpsc::Receiver<Vec<app_state::PluginCandidate>>>,
    scanning: bool,

    plugin_folder: String,
    found: Vec<app_state::PluginCandidate>,
    plugin_search: String,
    chain: Vec<ChainItem>,

    profiles: Vec<String>,
    sel_profile: String,
    new_profile_name: String,
    show_new_profile: bool,

    settings: AppSettings,
    show_settings: bool,

    /// Self-update worker results (never block the UI thread).
    update_rx: Option<mpsc::Receiver<UpdateMsg>>,
    update_state: UpdateUi,
    update_pending: Option<update::ReleaseInfo>,
    update_boot_checked: bool,
    /// Our own visibility tracking (egui has no visible flag): false
    /// after close-to-tray hides the window, true on show.
    visible: bool,

    cable_msg: String,
    toasts: toast::Toasts,
    /// Program-icon texture for the header (baked 256px RGBA).
    icon_tex: Option<egui::TextureHandle>,
    /// Snapshot undo/redo for chain edits (cap 50 each).
    undo: Vec<ChainSnap>,
    redo: Vec<ChainSnap>,
    peak_time: Instant,
    log_time: Instant,
    status_time: Instant,
    show_rx: mpsc::Receiver<()>,
    _tray: Option<tray::Tray>,
    guard: guard::CableGuard,
    /// Old C# app currently running (rechecked periodically).
    old_app_seen: bool,
    /// Whether we already asked it to close (second Start overrides).
    close_requested: bool,
    // Base64 states aligned with `chain` until hosts attach at start.
    pending_states: Vec<Option<String>>,
    /// Open editor windows as (chain index, raw HWND).
    editors: Vec<(usize, isize)>,
}

/// Self-update UI state machine (worker threads report via channel).
#[derive(Debug)]
enum UpdateUi {
    Idle,
    Checking,
    Available { tag: String, name: String },
    Downloading,
    Ready(std::path::PathBuf),
    Failed(String),
    Current,
}

#[derive(Debug)]
enum UpdateMsg {
    Check(Result<Option<update::ReleaseInfo>, String>),
    Download(Result<std::path::PathBuf, String>),
}

impl App {
    /// Fire-and-forget latest-release check on a worker thread.
    fn spawn_update_check(&mut self) {
        if self.update_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        self.update_state = UpdateUi::Checking;
        std::thread::spawn(move || {
            let _ = tx.send(UpdateMsg::Check(update::check_latest()));
        });
    }

    /// Fire-and-forget download+extract of a known release.
    fn spawn_update_download(&mut self, info: update::ReleaseInfo) {
        if self.update_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        self.update_state = UpdateUi::Downloading;
        std::thread::spawn(move || {
            let _ = tx.send(UpdateMsg::Download(update::download_and_stage(&info)));
        });
    }

    /// Collect finished worker results (non-blocking, every frame).
    fn poll_update(&mut self) {
        let msg = match self.update_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(m) => m,
            None => return,
        };
        self.update_rx = None;
        match msg {
            UpdateMsg::Check(Ok(None)) => {
                self.update_state = UpdateUi::Current;
            }
            UpdateMsg::Check(Ok(Some(rel))) => {
                self.update_state = UpdateUi::Available {
                    tag: rel.tag.clone(),
                    name: rel.name.clone(),
                };
                // Stash for the download button.
                self.update_pending = Some(rel);
                self.status = format!("update available: {}", self.update_tag());
            }
            UpdateMsg::Check(Err(e)) => {
                self.update_state = UpdateUi::Failed(e);
            }
            UpdateMsg::Download(Ok(stage)) => {
                self.update_state = UpdateUi::Ready(stage);
            }
            UpdateMsg::Download(Err(e)) => {
                self.update_state = UpdateUi::Failed(e);
            }
        }
    }

    fn update_tag(&self) -> String {
        match &self.update_state {
            UpdateUi::Available { tag, .. } => tag.clone(),
            _ => String::new(),
        }
    }
    fn new(_cc: &eframe::CreationContext<'_>, show_rx: mpsc::Receiver<()>, start_hidden: bool) -> Self {
        let session = load_session();
        // The app opens on the ACTIVE PROFILE when it exists — that's the
        // "main profile" the user saved or last switched to. The session
        // file is the fallback (no profile yet / profile deleted).
        let boot = {
            let active = session.ActiveProfile.clone();
            if !active.is_empty() && profile_names().contains(&active) {
                load_profile(&active).unwrap_or_else(|| session.clone())
            } else {
                session.clone()
            }
        };
        let settings = load_settings();
        let auto_start = settings.AutoStartMicrophone;
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default();
        let plugin_folder = if boot.PluginFolder.is_empty() {
            default_plugin_folder(&exe_dir).to_string_lossy().to_string()
        } else {
            boot.PluginFolder.clone()
        };
        let inputs = input_devices();
        let outputs = output_devices();
        let sel_input = pick_saved(&inputs, &boot.InputDeviceId, &boot.InputDeviceName);
        let mut sel_output = pick_saved(&outputs, &boot.OutputDeviceId, &boot.OutputDeviceName);
        if sel_output.is_empty() {
            // Auto-select the best-scored cable endpoint.
            sel_output = outputs
                .iter()
                .find(|(_, n)| n == "CABLE Input")
                .or_else(|| outputs.iter().find(|(_, n)| n.contains("CABLE Input")))
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
        }
        let found = scan(std::path::Path::new(&plugin_folder));
        log_line(&format!(
            "startup scan '{}': {} plugins ({})",
            plugin_folder,
            found.len(),
            found
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        let mut app = App {
            inputs,
            outputs,
            sel_input,
            sel_output,
            sel_block: if ALLOWED_BLOCKS.contains(&boot.BufferSize) {
                boot.BufferSize
            } else {
                1024
            },
            route: None,
            status: "IDLE".into(),
            status_diag: "Engine idle.".into(),
            scan_rx: None,
            scanning: false,
            plugin_folder,
            found,
            plugin_search: String::new(),
            chain: boot
                .Plugins
                .iter()
                .map(|p| ChainItem {
                    name: p.Name.clone(),
                    path: std::path::PathBuf::from(&p.Path),
                    format: if p.Format.eq_ignore_ascii_case("Vst3") {
                        PluginFormat::Vst3
                    } else {
                        PluginFormat::Vst2
                    },
                    enabled: p.Enabled,
                    error: None,
                    attached: false,
                })
                .collect(),
            profiles: profile_names(),
            // A session can outlive its profile (deleted meanwhile):
            // never boot with a ghost selection.
            sel_profile: {
                let want = boot.ActiveProfile.clone();
                if want.is_empty() || !profile_names().contains(&want) {
                    String::new()
                } else {
                    want
                }
            },
            new_profile_name: String::new(),
            show_new_profile: false,
            cable_msg: String::new(),
            toasts: toast::Toasts::default(),
            icon_tex: None,
            undo: Vec::new(),
            redo: Vec::new(),
            peak_time: Instant::now(),
            log_time: Instant::now(),
            status_time: Instant::now(),
            show_rx,
            _tray: None,
            guard: guard::CableGuard::new(),
            old_app_seen: conflict::old_app_running(),
            close_requested: false,
            settings,
            show_settings: false,
            update_rx: None,
            update_state: UpdateUi::Idle,
            update_pending: None,
            update_boot_checked: false,
            visible: !start_hidden,
            pending_states: Vec::new(),
            editors: Vec::new(),
        };
        app._tray = Some(tray::Tray::new(_cc.egui_ctx.clone()));
        app.refresh_cable_status();
        // Restore per-plugin states into pending slots via session re-load on start.
        app.pending_states = boot.Plugins.iter().map(|p| p.State.clone()).collect();
        if auto_start {
            app.try_start();
        }
        app
    }

    fn exe_dir() -> std::path::PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default()
    }

    fn host_dll(&self, format: PluginFormat) -> std::path::PathBuf {
        Self::exe_dir().join(format.dll_name())
    }

    fn refresh_devices(&mut self) {
        self.inputs = input_devices();
        self.outputs = output_devices();
        self.refresh_cable_status();
    }

    /// Directory scan on a worker thread: large plugin folders must
    /// never freeze the UI. Results land via `poll_scan` each frame.
    fn start_scan(&mut self) {
        if self.scanning {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let folder = self.plugin_folder.clone();
        std::thread::spawn(move || {
            let found = scan(std::path::Path::new(&folder));
            let _ = tx.send(found);
        });
        self.scan_rx = Some(rx);
        self.scanning = true;
        self.status = "Scanning plugins…".into();
    }

    /// Collect a finished background scan (non-blocking).
    fn poll_scan(&mut self) {
        if let Some(rx) = &self.scan_rx {
            if let Ok(found) = rx.try_recv() {
                self.status = format!("Scan: {} plugins", found.len());
                log_line(&format!("scan done: {} plugins", found.len()));
                self.found = found;
                self.scan_rx = None;
                self.scanning = false;
            }
        }
    }

    fn refresh_cable_status(&mut self) {
        let names: Vec<&str> = self.outputs.iter().map(|(_, n)| n.as_str()).collect();
        self.cable_msg = guard::cable_status(&names).1;
    }

    fn try_start(&mut self) {
        // Mutual exclusion: never route into a guard war with the old app.
        // First press asks it to close; a second press overrides.
        if conflict::old_app_running() {
            self.old_app_seen = true;
            if !self.close_requested {
                let n = conflict::quit_old_app();
                self.close_requested = true;
                self.status = format!(
                    "old VST-Applier is running (asked {n} window(s) to close) — press Start again in a couple seconds"
                );
                log_line("START refused: old app running, close requested");
                return;
            }
            log_line("START override: old app still running");
        }
        self.stop_route();
        log_line(&format!(
            "START in='{}' out='{}' blk={} chain_items={}",
            self.sel_input,
            self.sel_output,
            self.sel_block,
            self.chain.len()
        ));
        let mut plugins: Vec<ChainPlugin> = Vec::new();
        let mut attach_out: Vec<(usize, Option<String>, bool)> = Vec::new();
        for (i, item) in self.chain.iter().enumerate() {
            let state = self
                .pending_states
                .get(i)
                .and_then(|s| s.as_deref())
                .and_then(|s| decode_state_field(Some(s)).ok())
                .flatten();
            let mut cp = ChainPlugin {
                name: item.name.clone(),
                path: item.path.clone(),
                format: item.format,
                enabled: item.enabled,
                host: None,
                saved_state: state,
            };
            if item.enabled {
                let dll = self.host_dll(item.format);
                if let Err(e) = cp.attach(&dll) {
                    self.status = format!("plugin '{}' failed to load: {e}", item.name);
                    log_line(&format!("plugin '{}' load failed: {e}", item.name));
                    attach_out.push((i, Some(e.to_string()), false));
                } else {
                    attach_out.push((i, None, true));
                }
            } else {
                attach_out.push((i, None, false));
            }
            plugins.push(cp);
        }
        for (i, err, att) in attach_out {
            if let Some(item) = self.chain.get_mut(i) {
                item.error = err;
                item.attached = att;
            }
        }
        match start(&self.sel_input, &self.sel_output, plugins, self.sel_block) {
            Ok(route) => {
                self.status = route.human_line();
                self.status_diag = route.status_line();
                log_line(&format!("route running: {}", self.status_diag));
                // Routing audibly means "unmute me": clear any persisted
                // self-mute left over from the old-vs-new guard war.
                guard::unmute_own_sessions(Some(&self.sel_output));
                self.route = Some(route);
            }
            Err(e) => {
                self.status = format!("start failed: {e}");
                log_line(&format!("start failed: {e}"));
            }
        }
    }

    /// Tell every open editor's host first, then destroy the windows.
    /// Must run before the route (and its hosts) is dropped.
    fn close_all_editors(&mut self) {
        for (idx, raw) in std::mem::take(&mut self.editors) {
            if let Some(route) = self.route.as_ref() {
                let _ = route.with_plugin(idx, |p| {
                    if let Some(h) = p.host.as_mut() {
                        let _ = h.close_editor();
                    }
                });
            }
            editor::close(raw);
        }
    }

    fn stop_route(&mut self) {
        self.close_all_editors();
        if let Some(route) = self.route.take() {
            // Harvest plugin states back into pending slots (by order).
            let blobs = route.stop();
            self.pending_states = blobs.into_iter().map(|b| encode_state_field(b.as_deref())).collect();
            // Pad in case the chain changed mid-flight.
            while self.pending_states.len() < self.chain.len() {
                self.pending_states.push(None);
            }
            for item in self.chain.iter_mut() {
                item.attached = false;
            }
        }
    }

    fn restart_if_running(&mut self) {
        if self.route.is_some() {
            self.try_start();
        }
    }

    /// Toggle the editor window for chain slot `i`.
    fn toggle_editor(&mut self, i: usize) {
        if let Some(pos) = self.editors.iter().position(|(idx, _)| *idx == i) {
            let (_, raw) = self.editors.remove(pos);
            if let Some(route) = self.route.as_ref() {
                let _ = route.with_plugin(i, |p| {
                    if let Some(h) = p.host.as_mut() {
                        let _ = h.close_editor();
                    }
                });
            }
            editor::close(raw);
            return;
        }
        let Some(route) = self.route.as_ref() else {
            self.status = "start the route first, then open editors".into();
            return;
        };
        // Preferred size from the host (VST2); sensible default otherwise.
        let size = route
            .with_plugin(i, |p| p.host.as_mut().and_then(|h| h.editor_size()))
            .flatten()
            .unwrap_or((520, 420));
        let name = self.chain.get(i).map(|c| c.name.clone()).unwrap_or_default();
        match editor::open_editor_window(&format!("{name} — editor"), size.0, size.1) {
            Ok(raw) => {
                let opened = route
                    .with_plugin(i, |p| {
                        p.host.as_mut().map(|h| unsafe { h.open_editor(editor::hwnd_raw(raw)) })
                    })
                    .flatten();
                match opened {
                    Some(Ok(())) => {
                        self.editors.push((i, raw));
                        log_line(&format!("editor opened for '{name}'"));
                    }
                    _ => {
                        editor::close(raw);
                        self.status = format!("'{name}' has no editor view");
                    }
                }
            }
            Err(e) => self.status = format!("editor window failed: {e}"),
        }
    }

    fn apply_profile(&mut self, name: &str) {
        let Some(p) = load_profile(name) else {
            self.status = format!("profile '{name}' not found");
            return;
        };
        // Harvest BEFORE mutating: route.stop() overwrites
        // pending_states in route order, so stopping after the swap
        // would misalign every slot's settings.
        let was_running = self.route.is_some();
        self.stop_route();
        if self.inputs.iter().any(|(id, _)| id == &p.InputDeviceId) {
            self.sel_input = p.InputDeviceId.clone();
        } else if self.inputs.iter().any(|(_, n)| n == &p.InputDeviceName) {
            self.sel_input = self.inputs.iter().find(|(_, n)| n == &p.InputDeviceName).map(|(i, _)| i.clone()).unwrap_or_default();
        }
        if self.outputs.iter().any(|(id, _)| id == &p.OutputDeviceId) {
            self.sel_output = p.OutputDeviceId.clone();
        } else if self.outputs.iter().any(|(_, n)| n == &p.OutputDeviceName) {
            self.sel_output = self.outputs.iter().find(|(_, n)| n == &p.OutputDeviceName).map(|(i, _)| i.clone()).unwrap_or_default();
        }
        if ALLOWED_BLOCKS.contains(&p.BufferSize) {
            self.sel_block = p.BufferSize;
        }
        if !p.PluginFolder.is_empty() {
            self.plugin_folder = p.PluginFolder.clone();
            self.found = scan(std::path::Path::new(&self.plugin_folder));
        }
        // Undo covers the chain only (devices stay as applied).
        self.checkpoint();
        self.chain = p
            .Plugins
            .iter()
            .map(|e| ChainItem {
                name: e.Name.clone(),
                path: std::path::PathBuf::from(&e.Path),
                format: if e.Format.eq_ignore_ascii_case("Vst3") {
                    PluginFormat::Vst3
                } else {
                    PluginFormat::Vst2
                },
                enabled: e.Enabled,
                error: None,
                attached: false,
            })
            .collect();
        self.pending_states = p.Plugins.iter().map(|e| e.State.clone()).collect();
        self.status = format!("loaded '{name}'");
        if was_running {
            self.try_start();
        }
    }

    /// Reset just the chain to the selected profile's saved order/plugins
    /// (devices and buffer stay as-is).
    fn load_profile_order(&mut self) {
        if self.sel_profile.is_empty() {
            self.status = "Load order: no profile selected".into();
            return;
        }
        // Same harvest-first ordering as apply_profile: stop the old
        // route before swapping, then restart only if we were live.
        let name = self.sel_profile.clone();
        let Some(p) = load_profile(&name) else {
            self.status = format!("profile '{name}' not found");
            return;
        };
        let was_running = self.route.is_some();
        self.stop_route();
        self.close_all_editors();
        self.checkpoint();
        let (chain, states) = chain_from_profile(&p);
        let n = chain.len();
        self.chain = chain;
        self.pending_states = states;
        self.status = format!("chain reset to '{name}' order ({n} plugins)");
        log_line(&format!("Load order: chain reset to '{name}' ({n} plugins)"));
        if was_running {
            self.try_start();
        }
    }

    /// Snapshot the chain before a structural edit. Call AFTER stop
    /// (pending is freshly harvested) and BEFORE mutating — restores
    /// then replays the harvest-first restart like every other edit.
    fn checkpoint(&mut self) {
        self.redo.clear();
        self.undo.push(ChainSnap {
            chain: self.chain.clone(),
            pending: self.pending_states.clone(),
        });
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
    }

    fn undo_chain(&mut self) {
        let Some(snap) = self.undo.pop() else {
            self.status = "nothing to undo".into();
            return;
        };
        // Harvest first so the redo snapshot carries fresh states.
        let was_running = self.route.is_some();
        if was_running {
            self.stop_route();
        }
        self.redo.push(ChainSnap {
            chain: self.chain.clone(),
            pending: self.pending_states.clone(),
        });
        if self.redo.len() > 50 {
            self.redo.remove(0);
        }
        self.chain = snap.chain;
        self.pending_states = snap.pending;
        self.status = "undone".into();
        self.toasts.ok("undone");
        if was_running {
            self.try_start();
        }
    }

    fn redo_chain(&mut self) {
        let Some(snap) = self.redo.pop() else {
            self.status = "nothing to redo".into();
            return;
        };
        let was_running = self.route.is_some();
        if was_running {
            self.stop_route();
        }
        self.undo.push(ChainSnap {
            chain: self.chain.clone(),
            pending: self.pending_states.clone(),
        });
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
        self.chain = snap.chain;
        self.pending_states = snap.pending;
        self.status = "redone".into();
        self.toasts.ok("redone");
        if was_running {
            self.try_start();
        }
    }

    /// Save the selected profile (header button + Ctrl+S share this).
    fn save_current(&mut self) {
        // Never write ".json": saving requires a name.
        if self.sel_profile.is_empty() {
            self.status = "Save: no profile selected — create one first".into();
            return;
        }
        let p = self.capture_profile(&self.sel_profile);
        match save_profile(&p) {
            Ok(()) => {
                // The saved profile becomes the one the app opens on:
                // mirror it into the session right now (exit-time save
                // would lose it on a crash/force-kill).
                let _ = save_session(&p);
                let states = p.Plugins.iter().filter(|e| e.State.is_some()).count();
                self.status =
                    format!("saved '{}' — {} plugins, {states} with settings", p.Name, p.Plugins.len());
                log_line(&format!(
                    "profile saved '{}': {} plugins, {states} with settings",
                    p.Name,
                    p.Plugins.len()
                ));
                self.toasts.ok(format!("saved '{}'", p.Name));
            }
            Err(e) => {
                self.status = format!("save failed: {e}");
                self.toasts.warn(format!("save failed: {e}"));
            }
        }
        self.profiles = profile_names();
    }

    fn capture_profile(&self, name: &str) -> VoiceSetupProfile {
        // Live harvest: while the route runs, query each attached host for
        // its CURRENT state. Knobs turned after the last stop would
        // otherwise be captured stale — the "sometimes it doesn't save my
        // settings" bug. Fall back to the last harvested blob.
        let live_state = |i: usize| -> Option<String> {
            let route = self.route.as_ref()?;
            let blob: Option<Vec<u8>> = route
                .try_with_plugin(i, |p| {
                    p.host
                        .as_mut()
                        .and_then(|h| h.save_state().ok().flatten())
                })
                .flatten();
            encode_state_field(blob.as_deref())
        };
        VoiceSetupProfile {
            SchemaVersion: 1,
            Name: name.into(),
            ActiveProfile: name.into(),
            PluginFolder: self.plugin_folder.clone(),
            InputDeviceId: self.sel_input.clone(),
            InputDeviceName: display_name(&self.inputs, &self.sel_input),
            OutputDeviceId: self.sel_output.clone(),
            OutputDeviceName: display_name(&self.outputs, &self.sel_output),
            BufferSize: self.sel_block,
            Plugins: self
                .chain
                .iter()
                .enumerate()
                .map(|(i, c)| app_state::VoiceSetupPluginEntry {
                    Name: c.name.clone(),
                    Path: c.path.to_string_lossy().to_string(),
                    Format: match c.format {
                        PluginFormat::Vst2 => "Vst2".into(),
                        PluginFormat::Vst3 => "Vst3".into(),
                    },
                    Enabled: c.enabled,
                    State: live_state(i)
                        .or_else(|| self.pending_states.get(i).and_then(|s| s.clone())),
                })
                .collect(),
        }
    }
}

/// Build chain items + pending states from a saved profile.
fn chain_from_profile(p: &VoiceSetupProfile) -> (Vec<ChainItem>, Vec<Option<String>>) {
    let chain = p
        .Plugins
        .iter()
        .map(|e| ChainItem {
            name: e.Name.clone(),
            path: std::path::PathBuf::from(&e.Path),
            format: if e.Format.eq_ignore_ascii_case("Vst3") {
                PluginFormat::Vst3
            } else {
                PluginFormat::Vst2
            },
            enabled: e.Enabled,
            error: None,
            attached: false,
        })
        .collect();
    let states = p.Plugins.iter().map(|e| e.State.clone()).collect();
    (chain, states)
}

fn log_line(msg: &str) {
    let path = app_state::settings_dir().join("rust-log.txt");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write as _;
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}

fn pick_saved(devs: &[(String, String)], id: &str, name: &str) -> String {
    if !id.is_empty() {
        if let Some((i, _)) = devs.iter().find(|(i, _)| i == id) {
            return i.clone();
        }
    }
    if !name.is_empty() {
        if let Some((i, _)) = devs.iter().find(|(_, n)| n == name) {
            return i.clone();
        }
    }
    String::new()
}

fn display_name(devs: &[(String, String)], id: &str) -> String {
    devs.iter()
        .find(|(i, _)| i == id)
        .map(|(_, n)| n.clone())
        .unwrap_or_default()
}

/// Fixed 396x30 Audio Route row: icon + label column + selector share
/// one centered baseline. Fixed widths only — never `available_width`
/// inside a SidePanel (early-frame reads ratchet the panel open).
#[allow(dead_code)]
fn route_row(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(396.0, 30.0),
        egui::Layout::left_to_right(egui::Align::Center),
        f,
    );
}

/// Fixed label column so all route selectors start at the same x.
#[allow(dead_code)]
fn route_lab(ui: &mut egui::Ui, text: &str) {
    ui.scope(|ui| {
        ui.set_min_width(118.0);
        theme::hud_colored(ui, text, theme::MUTED);
    });
}

/// Middle-elide long device names: the selector button sizes to its
/// text, so an unclipped 40-char device name would blow out the fixed
/// row. Full names stay visible in the popup list and the tooltip.
fn elide_middle(s: &str) -> String {
    const MAX: usize = 30;
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= MAX {
        return s.to_string();
    }
    let head: String = chars[..22].iter().collect();
    let tail: String = chars[chars.len() - 7..].iter().collect();
    format!("{head}…{tail}")
}

/// LED-segment level row: fixed label, meter fills the resting width,
/// 1-decimal readout right-aligned at the row end.
fn level_row(ui: &mut egui::Ui, label: &str, level: f32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (lr, _) = ui.allocate_exact_size(egui::vec2(54.0, 20.0), egui::Sense::hover());
        ui.painter().text(
            lr.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            theme::TEXT_DIM,
        );
        let db = if level <= 0.0 {
            "-∞ dB".to_string()
        } else {
            format!("{:+.1} dB", 20.0 * level.log10())
        };
        // Fixed-width value slot, sized from a reference string — never
        // from the live value: per-row text widths made the two meters
        // different lengths ('one bigger than the other makes the look
        // bad'). The number itself is still drawn right-aligned.
        let slot_w = ui
            .painter()
            .layout_no_wrap(
                "-00.0 dB".to_string(),
                egui::FontId::monospace(13.0),
                egui::Color32::WHITE,
            )
            .size()
            .x;
        let mw = (ui.available_width() - slot_w - 22.0).max(60.0);
        let (mr, _) = ui.allocate_exact_size(egui::vec2(mw, 18.0), egui::Sense::hover());
        // Sunken well so the segments read as one meter, not floating dashes.
        let bed = mr.expand2(egui::vec2(4.0, 3.0));
        ui.painter().rect(
            bed,
            egui::CornerRadius::ZERO,
            theme::SUNKEN,
            egui::Stroke::new(1.0_f32, theme::LINE),
            egui::StrokeKind::Inside,
        );
        // -60 dB floor.
        let pos = if level <= 0.0 {
            0.0
        } else {
            ((level.max(1e-4).log10() * 20.0 + 60.0) / 60.0).clamp(0.0, 1.0)
        };
        const SEGS: usize = 30;
        const GAP: f32 = 2.0;
        let sw = (mr.width() - GAP * (SEGS as f32 - 1.0)) / SEGS as f32;
        let lit = (pos * SEGS as f32).round() as usize;
        for i in 0..SEGS {
            let x = mr.min.x + i as f32 * (sw + GAP);
            let r =
                egui::Rect::from_min_size(egui::pos2(x, mr.min.y), egui::vec2(sw, mr.height()));
            let f = i as f32 / SEGS as f32;
            let c = if i < lit {
                if f < 0.62 {
                    theme::GOOD
                } else if f < 0.85 {
                    theme::WARN
                } else {
                    theme::BAD
                }
            } else {
                egui::Color32::from_rgb(0x1b, 0x1c, 0x1f)
            };
            ui.painter()
                .rect_filled(r, egui::CornerRadius::same(2), c);
        }
        let db_galley = ui.painter().layout_no_wrap(
            db,
            egui::FontId::monospace(13.0),
            egui::Color32::WHITE,
        );
        let (dr, _) = ui.allocate_exact_size(
            egui::vec2((slot_w + 12.0).min(ui.available_width()), 20.0),
            egui::Sense::hover(),
        );
        ui.painter().galley(
            egui::pos2(
                dr.right() - db_galley.size().x,
                dr.center().y - db_galley.size().y / 2.0,
            ),
            db_galley,
            egui::Color32::WHITE,
        );
    });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Cache the main window handle: the tray / single-instance paths
        // use it to reveal the window at the OS level, which is the only
        // wake that always works while the window is hidden.
        {
            use raw_window_handle::HasWindowHandle as _;
            if let Ok(h) = _frame.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(w) = h.as_raw() {
                    win::set_main_hwnd(w.hwnd.get());
                }
            }
        }
        // Program-icon texture for the header (baked RGBA, no decoder).
        if self.icon_tex.is_none() {
            const SRC: usize = 256;
            let raw: &[u8] = include_bytes!("../assets/icon-256.rgba");
            // The master is mostly dark tile with the logo in the middle
            // (content bbox x39-234, y40-216): trim to the logo + a 6px
            // breath, or the header icon reads ~30% smaller than drawn.
            let (cx, cy, cw, ch) = (33usize, 34usize, 207usize, 188usize);
            let mut px = Vec::with_capacity(cw * ch * 4);
            for y in cy..cy + ch {
                let s = (y * SRC + cx) * 4;
                px.extend_from_slice(&raw[s..s + cw * 4]);
            }
            let img = egui::ColorImage::from_rgba_unmultiplied([cw, ch], &px);
            self.icon_tex = Some(ctx.load_texture(
                "app_icon",
                img,
                egui::TextureOptions::LINEAR,
            ));
        }
        // Second-instance wake requests.
        while self.show_rx.try_recv().is_ok() {
            self.visible = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        // Keyboard shortcuts (never while typing in a field).
        let typing = ctx.wants_keyboard_input();
        if !typing && ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            if self.route.is_some() {
                self.stop_route();
            } else {
                self.try_start();
            }
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S)) {
            self.save_current();
        }
        if !typing && ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) {
            self.undo_chain();
        }
        if !typing
            && ctx.input(|i| {
                i.modifiers.ctrl
                    && (i.key_pressed(egui::Key::Y)
                        || (i.modifiers.shift && i.key_pressed(egui::Key::Z)))
            })
        {
            self.redo_chain();
        }
        // Collect a finished background plugin scan, if any.
        self.poll_scan();
        // Tray requests.
        if let Some(t) = self._tray.as_ref() {
            match t.poll() {
                tray::TrayAction::Open => {
                    // Mirror the second-instance wake path exactly: a
                    // minimized window ignores Visible(true) alone.
                    self.visible = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::TrayAction::Quit => {
                    self.stop_route();
                    let p = self.capture_profile(&self.sel_profile);
                    let _ = save_session(&p);
                    std::process::exit(0);
                }
                tray::TrayAction::None => {}
            }
        }
        // Collect finished self-update worker results.
        self.poll_update();
        // One automatic check per launch (opt-out in Settings).
        if !self.update_boot_checked {
            self.update_boot_checked = true;
            if self.settings.CheckUpdates {
                self.spawn_update_check();
            }
        }
        // Steady repaint for meters + old-app recheck about once a second.
        if self.peak_time.elapsed() > Duration::from_millis(1000) {
            self.peak_time = Instant::now();
            // Recheck the old-app conflict about once a second.
            self.old_app_seen = conflict::old_app_running();
            if !self.old_app_seen {
                self.close_requested = false;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(
            // Minimized: audio keeps flowing on the WASAPI threads, so
            // the UI idles at 1 Hz (tray menu + restore stay live).
            // Meters need full rate while running; an idle window has
            // nothing animating — 2 Hz keeps tray/clock state fresh.
            if ctx.input(|i| i.viewport().minimized).unwrap_or(false) {
                1000
            } else if self.route.is_some() {
                33
            } else {
                500
            },
        ));
        // Close-to-tray: intercept the X button, hide instead.
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.settings.CloseToTray {
                self.visible = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else {
                self.stop_route();
                let p = self.capture_profile(&self.sel_profile);
                let _ = save_session(&p);
            }
        }

        let (in_peak, out_peak) = self
            .route
            .as_ref()
            .map(|r| (r.meters().input(), r.meters().output()))
            .unwrap_or((0.0, 0.0));

        let running = self.route.is_some();
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            frame::title_drag_bg(ui);
            ui.add_space(4.0);
            // Fixed 34px strip, TOP-aligned cluster: every control
            // shares the same top edge, and every box is 34 tall, so
            // placement order cannot drift baselines by px.
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 34.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                let dot = if running {
                    theme::GOOD
                } else {
                    egui::Color32::from_gray(110)
                };
                if let Some(tex) = self.icon_tex.as_ref() {
                    // The program's own artwork, not a glyph. The texture
                    // is pre-trimmed to the logo; draw fills the 34px row.
                    ui.add(egui::Image::new((tex.id(), egui::vec2(35.0, 32.0))));
                }
                ui.label(
                    egui::RichText::new("VST-Applier")
                        .strong()
                        .size(19.0)
                        .color(egui::Color32::WHITE),
                );
                theme::badge(ui, if running { "Running" } else { "Idle" }, dot);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if frame::win_button(ui, frame::WinBtn::Close, false).clicked() {
                        if self.settings.CloseToTray {
                            self.visible = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        } else {
                            // Same path as the native X: persist + stop via
                            // on_exit, then close.
                            self.stop_route();
                            let p = self.capture_profile(&self.sel_profile);
                            let _ = save_session(&p);
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    if frame::win_button(
                        ui,
                        frame::WinBtn::MaxRestore,
                        frame::is_maximized(ctx),
                    )
                    .clicked()
                    {
                        frame::toggle_maximize(ctx);
                    }
                    if frame::win_button(ui, frame::WinBtn::Min, false).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    // Breathing room between app actions and OS chrome.
                    ui.add_space(10.0);
                    // Profile hub: ONE container for the whole profile
                    // workflow — pick (the app opens on it next launch),
                    // save, create, delete.
                    let prev = self.sel_profile.clone();
                    let hub_label = if self.sel_profile.is_empty() {
                        "No profile".to_string()
                    } else {
                        self.sel_profile.clone()
                    };
                    ui.scope(|ui| {
                        // Match the 34px header box height.
                        ui.spacing_mut().button_padding.y = 8.0;
                        ui.menu_button(
                            egui::RichText::new(format!(
                                "{}  {hub_label}",
                                icons::glyph(icons::Icon::Person)
                            ))
                            .size(14.0),
                            |ui| {
                                ui.set_min_width(208.0);
                                // Cap the width: near the screen edge the
                                // content ui's available_width() resolves
                                // to (screen right - menu left) ~ 385px, and
                                // rows sized from it stretch the popover
                                // into a big empty container. The cap makes
                                // every child hug a slim ~230px dropdown.
                                ui.set_max_width(214.0);
                                ui.add_space(2.0);
                                theme::hud(ui, "Profiles");
                                ui.add_space(4.0);
                                // Custom rows (no blue highlight): active =
                                // hairline fill + teal check, rest hover
                                // sunken. Capped + scrollable so a long
                                // roster stays tidy.
                                egui::ScrollArea::vertical()
                                    .id_salt("prof_menu")
                                    .max_height(230.0)
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        if self.profiles.is_empty() {
                                            ui.weak("none yet");
                                        }
                                        for name in self.profiles.clone() {
                                            let active = name == self.sel_profile;
                                            let (rect, resp) = ui.allocate_exact_size(
                                                egui::vec2(ui.available_width(), 24.0),
                                                egui::Sense::click(),
                                            );
                                            let p = ui.painter();
                                            if resp.hovered() && !active {
                                                p.rect_filled(
                                                    rect,
                                                    egui::CornerRadius::ZERO,
                                                    theme::SUNKEN,
                                                );
                                            }
                                            if active {
                                                p.rect_filled(
                                                    rect,
                                                    egui::CornerRadius::ZERO,
                                                    theme::LINE,
                                                );
                                            }
                                            let ink = if active {
                                                theme::INK_STRONG
                                            } else if resp.hovered() {
                                                theme::INK
                                            } else {
                                                theme::TEXT_DIM
                                            };
                                            if active {
                                                p.text(
                                                    egui::pos2(
                                                        rect.left() + 10.0,
                                                        rect.center().y,
                                                    ),
                                                    egui::Align2::LEFT_CENTER,
                                                    icons::glyph(icons::Icon::Check),
                                                    egui::FontId::proportional(13.0),
                                                    theme::GOOD,
                                                );
                                            }
                                            p.text(
                                                egui::pos2(
                                                    rect.left() + 32.0,
                                                    rect.center().y,
                                                ),
                                                egui::Align2::LEFT_CENTER,
                                                &name,
                                                egui::FontId::proportional(13.5),
                                                ink,
                                            );
                                            if resp.clicked() {
                                                self.sel_profile = name.clone();
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                ui.add_space(2.0);
                                ui.separator();
                                ui.add_space(4.0);
                                // Compact action rows on the roster's grid
                                // (glyph at 10, label at 32): keeps the menu
                                // a slim dropdown, not stacked button bars.
                                let action = |ui: &mut egui::Ui,
                                              glyph: &str,
                                              label: &str,
                                              base: egui::Color32,
                                              hover_ink: egui::Color32,
                                              enabled: bool|
                                 -> bool {
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 24.0),
                                        egui::Sense::click(),
                                    );
                                    let hovered = resp.hovered() && enabled;
                                    let p = ui.painter();
                                    if hovered {
                                        p.rect_filled(
                                            rect,
                                            egui::CornerRadius::ZERO,
                                            theme::SUNKEN,
                                        );
                                    }
                                    let ink = if !enabled {
                                        theme::FAINT
                                    } else if hovered {
                                        hover_ink
                                    } else {
                                        base
                                    };
                                    p.text(
                                        egui::pos2(rect.left() + 10.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        glyph,
                                        egui::FontId::proportional(13.0),
                                        ink,
                                    );
                                    p.text(
                                        egui::pos2(rect.left() + 32.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        label,
                                        egui::FontId::proportional(13.5),
                                        ink,
                                    );
                                    enabled && resp.clicked()
                                };
                                let has_split = !self.sel_profile.is_empty();
                                if action(
                                    ui,
                                    icons::glyph(icons::Icon::Disk),
                                    "Save now",
                                    theme::INK,
                                    theme::INK_STRONG,
                                    has_split,
                                ) {
                                    self.save_current();
                                    ui.close_menu();
                                }
                                if action(
                                    ui,
                                    icons::glyph(icons::Icon::Plus),
                                    "New profile…",
                                    theme::TEXT_DIM,
                                    theme::INK_STRONG,
                                    true,
                                ) {
                                    self.show_new_profile = true;
                                    ui.close_menu();
                                }
                                if action(
                                    ui,
                                    icons::glyph(icons::Icon::Trash),
                                    "Delete profile",
                                    theme::BAD,
                                    theme::BAD,
                                    has_split,
                                ) {
                                    let name = self.sel_profile.clone();
                                    let _ = delete_profile(&name);
                                    self.profiles = profile_names();
                                    self.sel_profile.clear();
                                    // Session without a profile: the
                                    // next boot falls back to it.
                                    let _ = save_session(&self.capture_profile(""));
                                    self.status = format!("deleted '{name}'");
                                    self.toasts.ok(format!("deleted '{name}'"));
                                    ui.close_menu();
                                }
                            },
                        );
                    });
                    if self.sel_profile != prev && !self.sel_profile.is_empty() {
                        let name = self.sel_profile.clone();
                        self.apply_profile(&name);
                    }
                });
            });
            if self.old_app_seen {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(
                            "Old VST-Applier is running — only one app can own the cable. Quit it from its tray icon (right-click its tray icon -> Exit).",
                        )
                        .color(egui::Color32::from_rgb(248, 113, 113))
                        .small(),
                    );
                    if ui.small_button("Quit old app").clicked() {
                        let n = conflict::quit_old_app();
                        self.close_requested = true;
                        self.status = format!("asked old app to close ({n} window(s))");
                        log_line("banner: close requested for old app");
                    }
                });
            }
        });

        egui::SidePanel::left("route")
            .resizable(true)
            .default_width(340.0)
            .width_range(300.0..=400.0)
            .show(ctx, |ui| {
                let in_name = display_name(&self.inputs, &self.sel_input);
                let out_name = display_name(&self.outputs, &self.sel_output);
                // Window-derived panel height (never content-derived):
                // used below to pin Settings to the sidebar bottom.
                let panel_h = ui.available_height();
                let panel_w = ui.available_width();
                {
                    static SIDEBAR_LOGGED: std::sync::atomic::AtomicBool =
                        std::sync::atomic::AtomicBool::new(false);
                    if !SIDEBAR_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        log_line(&format!("sidebar panel_w={panel_w:.0} panel_h={panel_h:.0}"));
                    }
                }
                let top = ui.next_widget_position().y;
                // Scroll fills the top of the rail (normal top-down flow);
                // the dock below absorbs leftover height so Settings sits
                // flush at the bottom without a gap.
                egui::ScrollArea::vertical()
                    .id_salt("left")
                    .max_height((panel_h - 108.0).max(120.0))
                    .show(ui, |ui| {
                let cable_status = self.cable_msg.clone();
                // Flat control rail: no cards, hairline-separated groups,
                // labels above fields (reads better in a narrow column),
                // one dominant Start / Stop.
                ui.horizontal(|ui| {
                    theme::hud_colored(ui, "Input", theme::MUTED);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            theme::badge(ui, &cable_status, theme::GOOD);
                        },
                    );
                });
                let full = ui.available_width();
                {
                    let (rr, _) = ui.allocate_exact_size(
                        egui::vec2(full, 30.0),
                        egui::Sense::hover(),
                    );
                    let mut cui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rr)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    cui.spacing_mut().button_padding.y = 8.0;
                    egui::ComboBox::from_id_salt("in_sel")
                        .width(full)
                        .height(300.0)
                        .selected_text(if in_name.is_empty() {
                            "(select...)".to_string()
                        } else {
                            elide_middle(&in_name)
                        })
                        .show_ui(&mut cui, |ui| {
                            for (id, name) in self.inputs.clone() {
                                ui.selectable_value(&mut self.sel_input, id.clone(), name);
                            }
                        })
                        .response
                        .on_hover_text(&in_name);
                }
                ui.add_space(10.0);
                theme::hud_colored(ui, "Output", theme::MUTED);
                {
                    let (rr, _) = ui.allocate_exact_size(
                        egui::vec2(full, 30.0),
                        egui::Sense::hover(),
                    );
                    let mut cui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rr)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    cui.spacing_mut().button_padding.y = 8.0;
                    egui::ComboBox::from_id_salt("out_sel")
                        .width(full)
                        .height(300.0)
                        .selected_text(if out_name.is_empty() {
                            "(select...)".to_string()
                        } else {
                            elide_middle(&out_name)
                        })
                        .show_ui(&mut cui, |ui| {
                            for (id, name) in self.outputs.clone() {
                                ui.selectable_value(&mut self.sel_output, id.clone(), name);
                            }
                        })
                        .response
                        .on_hover_text(&out_name);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    theme::hud_colored(ui, "Buffer", theme::MUTED);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let w = 120.0;
                            let (rr, _) = ui.allocate_exact_size(
                                egui::vec2(w, 30.0),
                                egui::Sense::hover(),
                            );
                            let mut cui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rr)
                                    .layout(egui::Layout::top_down(egui::Align::Min)),
                            );
                            cui.spacing_mut().button_padding.y = 8.0;
                            egui::ComboBox::from_id_salt("blk_sel")
                                .width(w)
                                .height(300.0)
                                .selected_text(format!("{}", self.sel_block))
                                .show_ui(&mut cui, |ui| {
                                    for &b in ALLOWED_BLOCKS {
                                        ui.selectable_value(
                                            &mut self.sel_block,
                                            b,
                                            format!("{b}"),
                                        );
                                    }
                                });
                        },
                    );
                });
                ui.add_space(12.0);
                // Hero action: one dominant full-width button.
                ui.horizontal(|ui| {
                    let bw = (ui.available_width() - 44.0).max(180.0);
                    if running {
                        if theme::button(ui, "Stop", theme::BtnKind::Danger, bw).clicked() {
                            self.stop_route();
                            self.status = "Stopped".into();
                            self.status_diag = "Engine idle.".into();
                        }
                    } else {
                        let ready = !self.sel_input.is_empty()
                            && !self.sel_output.is_empty();
                        ui.add_enabled_ui(ready, |ui| {
                            if theme::button(ui, "Start", theme::BtnKind::Primary, bw)
                                .on_disabled_hover_text(
                                    "Select an input and an output first",
                                )
                                .clicked()
                            {
                                self.try_start();
                            }
                        });
                    }
                    if icons::glyph_button(
                        ui,
                        icons::Glyph::Refresh,
                        "Refresh device lists",
                    )
                    .clicked()
                    {
                        self.refresh_devices();
                    }
                });
                ui.add_space(4.0);
                theme::hud(ui, "Tip — in Discord / Zoom / OBS");
                theme::hud(ui, "pick \"CABLE Output\" as your mic");
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                theme::hud_colored(ui, "Levels", theme::MUTED);
                ui.add_space(4.0);
                level_row(ui, "Input", in_peak);
                ui.add_space(4.0);
                level_row(ui, "Output", out_peak);
                if let Some(r) = self.route.as_ref() {
                    let (xr, _trim) = r.dropouts();
                    if xr > 0 {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let (r2, _) = ui.allocate_exact_size(
                                egui::vec2(8.0, 8.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(
                                egui::Rect::from_center_size(
                                    r2.center(),
                                    egui::vec2(6.0, 6.0),
                                ),
                                egui::CornerRadius::ZERO,
                                theme::WARN,
                            );
                            theme::hud_colored(
                                ui,
                                &format!("{xr} dropouts — try a larger buffer"),
                                theme::WARN,
                            );
                        });
                    }
                }
                }); // end left scroll
                // Dock: absorb leftover height so Settings sits flush at
                // the bottom (no big gap under it).
                ui.add_space((panel_h - (ui.next_widget_position().y - top) - 96.0).max(0.0));
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                let sresp = egui::Frame::default()
                    .fill(theme::BG)
                    .stroke(egui::Stroke::new(1.0_f32, theme::LINE))
                    .corner_radius(egui::CornerRadius::ZERO)
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 13,
                        bottom: 13,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icons::icon(ui, icons::Icon::Gear, icons::MIST);
                            let galley = {
                                let format = egui::TextFormat {
                                    font_id: egui::FontId::new(
                                        13.0,
                                        egui::FontFamily::Monospace,
                                    ),
                                    color: theme::INK_STRONG,
                                    extra_letter_spacing: 1.6,
                                    ..Default::default()
                                };
                                ui.painter().layout_job(
                                    egui::text::LayoutJob::single_section(
                                        "SETTINGS".to_string(),
                                        format,
                                    ),
                                )
                            };
                            ui.label(galley);
                            let gap = (ui.available_width() - 26.0).max(0.0);
                            ui.add_space(gap);
                            icons::icon(ui, icons::Icon::ChevR, theme::TEXT_DIM);
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if sresp.clicked() {
                    self.show_settings = true;
                }
                if let Some(r) = self.route.as_mut() {
                    r.poll_preload();
                    // Status strings rebuild at 5 Hz: meters stay live via
                    // atomics at full frame rate, but format! every frame
                    // (plus a status-mutex lock) is pure waste.
                    if self.status_time.elapsed() > Duration::from_millis(200) {
                        self.status_time = Instant::now();
                        self.status = r.human_line();
                        self.status_diag = r.status_line();
                    }
                    if self.log_time.elapsed() > Duration::from_millis(2000) {
                        self.log_time = Instant::now();
                        log_line(&format!(
                            "{} in_peak={:.3} out_peak={:.3}",
                            self.status,
                            r.meters().input(),
                            r.meters().output()
                        ));
                    }
                }
                // Keep the cable guard pointed at the selected output.
                self.guard.set_output(&self.sel_output);
                // Pump + reap editor windows (VST2 needs periodic idle).
                if !self.editors.is_empty() {
                    if let Some(route) = self.route.as_ref() {
                        for (idx, raw) in self.editors.clone() {
                            if !editor::is_open(raw) {
                                continue;
                            }
                            let _ = route.try_with_plugin(idx, |p| {
                                if let Some(h) = p.host.as_mut() {
                                    h.editor_idle();
                                }
                            });
                        }
                    }
                    // Reap windows the user closed directly.
                    let mut dead = Vec::new();
                    for (pos, (idx, raw)) in self.editors.iter().enumerate() {
                        if !editor::is_open(*raw) {
                            dead.push((pos, *idx));
                        }
                    }
                    for (pos, idx) in dead.into_iter().rev() {
                        self.editors.remove(pos);
                        if let Some(route) = self.route.as_ref() {
                            let _ = route.with_plugin(idx, |p| {
                                if let Some(h) = p.host.as_mut() {
                                    let _ = h.close_editor();
                                }
                            });
                        }
                    }
                }
            });

        // ---- Right rail: plugin library (search + full-height list +
        // folder). One Scan button, pinned at the bottom.
        egui::SidePanel::right("library")
            .resizable(true)
            .default_width(372.0)
            .width_range(320.0..=480.0)
            .show(ctx, |ui| {
                let top = ui.next_widget_position().y;
                let avail_h = ui.available_height();
                ui.horizontal(|ui| {
                    icons::icon(ui, icons::Icon::Puzzle, icons::CORAL);
                    theme::hud_colored(ui, "Plugins", theme::INK);
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            theme::hud(ui, &format!("{} found", self.found.len()));
                        },
                    );
                });
                ui.add_space(6.0);
                let sw = ui.available_width();
                ui.add_sized(
                    [sw, 30.0],
                    egui::TextEdit::singleline(&mut self.plugin_search)
                        .hint_text("Search plugins..."),
                );
                ui.add_space(6.0);
                let q = self.plugin_search.to_lowercase();
                egui::ScrollArea::vertical()
                    .id_salt("avail_list")
                    .max_height((ui.available_height() - 58.0).max(120.0))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let mut first = true;
                        for (k, c) in self.found.clone().into_iter().enumerate() {
                            if !q.is_empty()
                                && !c.name.to_lowercase().contains(&q)
                            {
                                continue;
                            }
                            if !first {
                                // Fixed 9px divider: exact row pitch.
                                let (dr, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 9.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().line_segment(
                                    [dr.left_center(), dr.right_center()],
                                    egui::Stroke::new(1.0_f32, theme::LINE),
                                );
                            }
                            first = false;
                            ui.push_id(("avail", k), |ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), 38.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.scope(|ui| {
                                            ui.set_min_width(80.0);
                                            match c.format {
                                                app_state::CandidateFormat::Vst3 => {
                                                    theme::badge(ui, "VST3", theme::GOOD);
                                                }
                                                app_state::CandidateFormat::Vst2 => {
                                                    theme::badge(ui, "VST2", theme::WARN);
                                                }
                                            }
                                        });
                                        ui.add_space(4.0);
                                        ui.label(
                                            egui::RichText::new(&c.name)
                                                .size(14.0)
                                                .color(theme::INK),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(
                                                egui::Align::Center,
                                            ),
                                            |ui| {
                                                if theme::button(
                                                    ui,
                                                    "Add",
                                                    theme::BtnKind::Secondary,
                                                    56.0,
                                                )
                                                .clicked()
                                                {
                                                    self.checkpoint();
                                                    self.chain.push(ChainItem {
                                                        name: c.name.clone(),
                                                        path: c.path.clone(),
                                                        format: match c.format {
                                                            app_state::CandidateFormat::Vst3 => {
                                                                PluginFormat::Vst3
                                                            }
                                                            app_state::CandidateFormat::Vst2 => {
                                                                PluginFormat::Vst2
                                                            }
                                                        },
                                                        enabled: true,
                                                        error: None,
                                                        attached: false,
                                                    });
                                                    self.pending_states.push(None);
                                                    self.restart_if_running();
                                                }
                                            },
                                        );
                                    },
                                );
                            });
                        }
                        if first {
                            ui.add_space(40.0);
                            ui.centered_and_justified(|ui| {
                                theme::hud(ui, "no plugins match");
                            });
                            ui.add_space(40.0);
                        }
                    });
                // Folder + Scan on their own footer shelf (hairline above),
                // so they read as panel chrome, not part of the plugin list.
                let used = ui.next_widget_position().y - top;
                ui.add_space((avail_h - used - 50.0).max(0.0));
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let fw = (ui.available_width() - 92.0).max(100.0);
                    // Display-only path chip: ellipsizes instead of cutting
                    // mid-word; hover shows the full path. Exact-size inner
                    // allocation, so the chip can never stretch or push the
                    // Scan button past the panel edge.
                    let path = self.plugin_folder.clone();
                    let box_resp = egui::Frame::default()
                        .fill(theme::SUNKEN)
                        .stroke(egui::Stroke::new(1.0_f32, theme::LINE))
                        .corner_radius(egui::CornerRadius::ZERO)
                        .inner_margin(egui::Margin {
                            left: 8,
                            right: 8,
                            top: 0,
                            bottom: 0,
                        })
                        .show(ui, |ui| {
                            let inner_w = fw - 16.0;
                            // Long paths: collapse the middle, keep the leaf
                            // folder ("C:\Program Files\…\Plugins") — the
                            // leaf is the part that identifies the folder.
                            let display = {
                                let measure = |t: &str| {
                                    ui.painter()
                                        .layout_no_wrap(
                                            t.to_string(),
                                            egui::FontId::monospace(11.5),
                                            theme::INK,
                                        )
                                        .size()
                                        .x
                                };
                                let mut s = path.clone();
                                if measure(&s) > inner_w - 8.0 {
                                    let parts: Vec<&str> = s.split('\\').collect();
                                    if parts.len() >= 3 {
                                        let leaf = parts[parts.len() - 1];
                                        for head_n in [3usize, 2, 1] {
                                            if head_n >= parts.len() - 1 {
                                                continue;
                                            }
                                            let head =
                                                parts[..head_n].join("\\");
                                            let cand =
                                                format!("{head}\\…\\{leaf}");
                                            if measure(&cand) <= inner_w - 8.0 {
                                                s = cand;
                                                break;
                                            }
                                        }
                                    }
                                }
                                s
                            };
                            ui.allocate_ui_with_layout(
                                egui::vec2(inner_w, 32.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&display)
                                                .monospace()
                                                .size(11.5)
                                                .color(theme::INK),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    );
                                },
                            );
                        })
                        .response
                        .interact(egui::Sense::hover());
                    box_resp.on_hover_text(format!("Plugins folder: {path}"));
                    let fscan = ui
                        .add_enabled_ui(!self.scanning, |ui| {
                            theme::button(
                                ui,
                                if self.scanning { "…" } else { "Scan" },
                                theme::BtnKind::Secondary,
                                76.0,
                            )
                        })
                        .inner
                        .on_hover_text("Rescan this folder for plugins");
                    if fscan.clicked() {
                        self.start_scan();
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            // Active chain FIRST — this is what you watch while routing.
            let mut mv: Option<(usize, isize)> = None;
            let mut rm: Option<usize> = None;
            let mut ed: Option<usize> = None;
            let mut changed = false;
            let undo_req = Cell::new(false);
            let redo_req = Cell::new(false);
            let clear_all = Cell::new(false);
            let load_order = Cell::new(false);
            let can_undo = !self.undo.is_empty();
            let can_redo = !self.redo.is_empty();
            let chain_sub = format!(
                "{}/{} plugins active",
                self.chain.iter().filter(|c| c.enabled).count(),
                self.chain.len(),
            );
            card2(
                ui,
                icons::Icon::Layers,
                "Plugin chain",
                Some(chain_sub.as_str()),
                Some(|ui: &mut egui::Ui| {
                    ui.add_enabled_ui(can_undo, |ui| {
                        if icons::glyph_button(ui, icons::Glyph::Undo, "Undo chain edit (Ctrl+Z)")
                            .clicked()
                        {
                            undo_req.set(true);
                        }
                    });
                    ui.add_enabled_ui(can_redo, |ui| {
                        if icons::glyph_button(ui, icons::Glyph::Redo, "Redo chain edit (Ctrl+Y)")
                            .clicked()
                        {
                            redo_req.set(true);
                        }
                    });
                    if theme::button(ui, "Reset order", theme::BtnKind::Secondary, 120.0)
                        .on_hover_text("Reset chain to the selected profile's order")
                        .clicked()
                    {
                        load_order.set(true);
                    }
                    if theme::button(ui, "Remove all", theme::BtnKind::Secondary, 120.0)
                        .on_hover_text("Remove every plugin from the chain")
                        .clicked()
                    {
                        clear_all.set(true);
                    }
                }),
                |ui| {
                        // Full-height chain list: a taller window shows
                        // more plugins (no fixed cap).
                        egui::ScrollArea::vertical()
                            .id_salt("chain")
                            .max_height((ui.available_height() - 44.0).max(160.0))
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                        for (i, item) in self.chain.iter_mut().enumerate() {
                            egui::Frame::default()
                                .fill(theme::SUNKEN)
                                .stroke(egui::Stroke::new(1.0_f32, theme::LINE))
                                .corner_radius(egui::CornerRadius::ZERO)
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    // Fixed-height strip with vertical
                                    // centering (horizontal_centered would
                                    // grab all available height).
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(ui.available_width(), 34.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                // Order number: hairline square, mono.
                                theme::tag(
                                    ui,
                                    &format!("{}", i + 1),
                                    theme::LINE,
                                    if item.enabled {
                                        theme::INK
                                    } else {
                                        theme::FAINT
                                    },
                                );
                                // Reference enable/disable checkbox.
                                if icons::checkbox_red(ui, &mut item.enabled) {
                                    changed = true;
                                }
                                // Format badge: teal for VST3, amber VST2.
                                match item.format {
                                    PluginFormat::Vst3 => {
                                        theme::badge(ui, "VST3", theme::GOOD);
                                    }
                                    PluginFormat::Vst2 => {
                                        theme::badge(ui, "VST2", theme::WARN);
                                    }
                                }
                                ui.label(
                                    egui::RichText::new(&item.name)
                                        .size(14.0)
                                        .color(if item.enabled {
                                            theme::INK
                                        } else {
                                            theme::FAINT
                                        }),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if icons::glyph_button(ui, icons::Glyph::X, "Remove").clicked() {
                                            rm = Some(i);
                                        }
                                        if icons::glyph_button(ui, icons::Glyph::Down, "Move down").clicked() {
                                            mv = Some((i, 1));
                                        }
                                        if icons::glyph_button(ui, icons::Glyph::Up, "Move up").clicked() {
                                            mv = Some((i, -1));
                                        }
                                        let ed_open = self.editors.iter().any(|(idx, _)| *idx == i);
                                        let ed_txt = if ed_open { "Edit •" } else { "Edit" };
                                        let ed_btn = ui.add_enabled_ui(item.attached, |ui| {
                                            theme::button(ui, ed_txt, theme::BtnKind::Secondary, 74.0)
                                        }).inner;
                                        let ed_btn = ed_btn.on_hover_text(if item.attached {
                                            "Open/close plugin editor"
                                        } else {
                                            "Start the route first"
                                        });
                                        if ed_btn.clicked() {
                                            ed = Some(i);
                                        }
                                    },
                                );
                                    });
                                });
                            ui.add_space(4.0);
                        }
                        });
                        if let Some((i, d)) = mv {
                            let j = (i as isize + d).clamp(0, self.chain.len() as isize - 1) as usize;
                            if j != i {
                                // Harvest first: stop_route() overwrites
                                // pending in route order, so the swap must
                                // come after.
                                let was_running = self.route.is_some();
                                self.stop_route();
                                self.checkpoint();
                                self.chain.swap(i, j);
                                let last = self.pending_states.len().saturating_sub(1);
                                if i <= last && j <= last {
                                    self.pending_states.swap(i, j);
                                }
                                for (idx, _) in self.editors.iter_mut() {
                                    if *idx == i {
                                        *idx = j;
                                    } else if *idx == j {
                                        *idx = i;
                                    }
                                }
                                if was_running {
                                    self.try_start();
                                }
                            }
                        }
                        if let Some(i) = rm {
                            let was_running = self.route.is_some();
                            self.stop_route();
                            self.checkpoint();
                            self.chain.remove(i);
                            if i < self.pending_states.len() {
                                self.pending_states.remove(i);
                            }
                            // Drop editors of the removed slot, shift higher ones.
                            self.editors.retain(|(idx, _)| *idx != i);
                            for (idx, _) in self.editors.iter_mut() {
                                if *idx > i {
                                    *idx -= 1;
                                }
                            }
                            if was_running {
                                self.try_start();
                            }
                        }
                        if let Some(i) = ed {
                            self.toggle_editor(i);
                        }
                        if changed {
                            // Order unchanged: harvest-then-flag is safe
                            // either way; keep the uniform pattern.
                            let was_running = self.route.is_some();
                            if was_running {
                                self.stop_route();
                            }
                            if was_running {
                                self.try_start();
                            }
                        }
                        if clear_all.get() {
                            // Stop first so the harvest can't repopulate
                            // pending after the clear (stale blobs would
                            // attach to future adds by index).
                            let was_running = self.route.is_some();
                            self.stop_route();
                            self.checkpoint();
                            self.chain.clear();
                            self.pending_states.clear();
                            self.status = "chain cleared".into();
                            self.toasts.ok("chain cleared");
                            if was_running {
                                self.try_start();
                            }
                        }
                        if load_order.get() {
                            self.load_profile_order();
                        }
                        if undo_req.get() {
                            self.undo_chain();
                        }
                        if redo_req.get() {
                            self.redo_chain();
                        }
                        if self.chain.is_empty() {
                            ui.add_space(40.0);
                            ui.centered_and_justified(|ui| {
                                theme::hud(ui, "chain is empty — add plugins below");
                            });
                            ui.add_space(40.0);
                        }
                    },
                ); // end Plugin chain card
        });

        egui::TopBottomPanel::bottom("status")
            .exact_height(34.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    theme::hud(ui, &self.status).on_hover_text(&self.status_diag);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme::hud(ui, &format!("v{}", update::APP_VERSION));
                    });
                });
            });

        // Borderless-frame edge grips (above panels, below dialogs).
        frame::edge_zones(ctx);

        // Toast stack (transient confirmations, above the status bar).
        self.toasts.show(ctx);

        if self.show_new_profile {
            egui::Window::new("Create profile").show(ctx, |ui| {
                ui.text_edit_singleline(&mut self.new_profile_name);
                ui.horizontal(|ui| {
                    if ui.button("Create").clicked() {
                        let name = self.new_profile_name.trim().to_string();
                        if is_valid_profile_name(&name) {
                            // The new profile captures the CURRENT setup
                            // (devices + chain) as its starting point —
                            // nothing is wiped, the user curates from here.
                            // It immediately becomes the profile the app
                            // opens on (session mirrors it now, not at exit).
                            self.sel_profile = name.clone();
                            let p = self.capture_profile(&name);
                            let _ = save_profile(&p);
                            let _ = save_session(&p);
                            log_line(&format!(
                                "profile created '{name}': {} plugins",
                                p.Plugins.len()
                            ));
                            self.profiles = profile_names();
                            self.status = format!("created '{name}' — opens on it next launch");
                            self.toasts.ok(format!("created '{name}'"));
                        } else {
                            self.status = "invalid profile name".into();
                        }
                        self.new_profile_name.clear();
                        self.show_new_profile = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.show_new_profile = false;
                    }
                });
            });
        }

        if self.show_settings {
            egui::Window::new("Settings").show(ctx, |ui| {
                let was_running = self.route.is_some();
                ui.checkbox(&mut self.settings.AutoStartMicrophone, "Start microphone routing on open");
                ui.checkbox(&mut self.settings.CloseToTray, "Keep running in tray on close");
                ui.checkbox(&mut self.settings.StartWithWindows, "Start with Windows (minimized to tray)");
                ui.checkbox(&mut self.settings.KeepCableClean, "Mute other apps on the virtual cable");
                ui.checkbox(&mut self.settings.CheckUpdates, "Check GitHub for updates on startup");
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!("Version {}", update::APP_VERSION));
                    let busy = self.update_rx.is_some();
                    ui.add_enabled_ui(!busy, |ui| {
                        if ui.button("Check now").clicked() {
                            self.spawn_update_check();
                        }
                    });
                });
                match &self.update_state {
                    UpdateUi::Idle => {}
                    UpdateUi::Checking => {
                        ui.label("Checking GitHub…");
                    }
                    UpdateUi::Current => {
                        ui.label("You're on the latest version.");
                    }
                    UpdateUi::Failed(e) => {
                        ui.label(format!("Update check failed: {e}"));
                    }
                    UpdateUi::Available { tag, name, .. } => {
                        ui.label(format!("{name} ({tag}) is available."));
                        // Clone out of the borrow before spawning.
                        let pending = self.update_pending.clone();
                        if ui.button("Download update").clicked() {
                            if let Some(info) = pending {
                                self.spawn_update_download(info);
                            }
                        }
                    }
                    UpdateUi::Downloading => {
                        ui.label("Downloading + extracting…");
                    }
                    UpdateUi::Ready(stage) => {
                        ui.label("Update staged — the app restarts to finish.");
                        let stage = stage.clone();
                        if ui.button("Install + Restart").clicked() {
                            self.status = update::install_and_restart(&stage);
                        }
                    }
                }
                if ui.button("Save").clicked() {
                    let _ = save_settings(&self.settings);
                    autostart::apply(self.settings.StartWithWindows);
                    self.guard.set_enabled(self.settings.KeepCableClean);
                    let _ = was_running;
                    self.show_settings = false;
                }
            });
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_route();
        let p = self.capture_profile(&self.sel_profile);
        let _ = save_session(&p);
    }
}

