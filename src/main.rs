// GUI subsystem: Windows never allocates a console, so double-click
// launches show zero terminal flash. (Probes like --sessions still
// print fine when run from an existing console: no-new-console only
// means "don't create one". File logging is unaffected.)
#![windows_subsystem = "windows"]

// NOTE: the subsystem stays console (not windows_subsystem) on purpose:
// CLI probes (--sessions etc.) need stdout. The console is hidden
// below for normal GUI launches instead.

mod autostart;
mod conflict;
mod editor;
mod frame;
mod guard;
mod icons;
mod tray;
mod update;

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

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VST-Applier")
            .with_inner_size(initial_size())
            .with_min_inner_size([1024.0, 700.0])
            .with_decorations(false)
            .with_icon(window_icon())
            .with_visible(!start_hidden),
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

/// Near-black charcoal with navy undertones (reference look).
fn app_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    let teal = egui::Color32::from_rgb(45, 212, 191);
    let card = egui::Color32::from_rgb(20, 26, 36);
    v.window_fill = egui::Color32::from_rgb(12, 16, 22);
    v.panel_fill = egui::Color32::from_rgb(12, 16, 22);
    v.faint_bg_color = egui::Color32::from_rgb(26, 32, 44);
    v.extreme_bg_color = egui::Color32::from_rgb(15, 21, 31);
    v.code_bg_color = egui::Color32::from_rgb(8, 11, 16);
    v.window_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(48, 60, 82));
    v.selection.bg_fill = egui::Color32::from_rgb(13, 84, 76);
    v.selection.stroke = egui::Stroke::new(1.0_f32, teal);
    v.hyperlink_color = teal;
    for state in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        state.corner_radius = egui::CornerRadius::same(6);
    }
    v.widgets.noninteractive.bg_fill = card;
    v.widgets.inactive.bg_fill = egui::Color32::from_rgb(24, 31, 44);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(214, 228, 245));
    v.widgets.hovered.bg_fill = egui::Color32::from_rgb(33, 42, 59);
    v.widgets.active.bg_fill = egui::Color32::from_rgb(13, 84, 76);
    v.window_corner_radius = egui::CornerRadius::same(10);
    v.menu_corner_radius = egui::CornerRadius::same(8);
    v
}

/// Full style: our visuals plus the reference type scale and spacing
/// (`text_styles`/`spacing` live on `Style`, not `Visuals`).
fn app_style() -> egui::Style {
    use egui::{FontFamily, FontId, TextStyle};
    let mut s = egui::Style {
        visuals: app_visuals(),
        ..Default::default()
    };
    // Reference proportions: larger, airier type and spacing.
    s.text_styles.insert(
        TextStyle::Heading,
        FontId::new(24.0, FontFamily::Proportional),
    );
    s.text_styles.insert(
        TextStyle::Body,
        FontId::new(16.0, FontFamily::Proportional),
    );
    s.text_styles.insert(
        TextStyle::Button,
        FontId::new(16.0, FontFamily::Proportional),
    );
    s.text_styles.insert(
        TextStyle::Small,
        FontId::new(13.0, FontFamily::Proportional),
    );
    s.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.5, FontFamily::Monospace),
    );
    s.spacing.item_spacing = egui::vec2(10.0, 8.0);
    s.spacing.button_padding = egui::vec2(12.0, 8.0);
    s.spacing.combo_height = 300.0;
    s
}

/// Card container: charcoal panel, red section icon, title + subtitle,
/// optional header-right widgets. Never size content from
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
        .fill(egui::Color32::from_rgb(20, 26, 36))
        .stroke(egui::Stroke::new(
            1.0_f32,
            egui::Color32::from_rgb(48, 60, 82),
        ))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(14))
        .outer_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 6,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                icons::icon(ui, ic, icons::CORAL);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(title)
                            .strong()
                            .size(17.0)
                            .color(egui::Color32::WHITE),
                    );
                    if let Some(sub) = subtitle {
                        ui.label(
                            egui::RichText::new(sub)
                                .size(13.0)
                                .color(egui::Color32::from_rgb(165, 180, 200)),
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
        (w as f32 * 0.66).clamp(1024.0, 1240.0),
        (h as f32 * 0.72).clamp(700.0, 840.0),
    ]
}

/// Returns the show-window channel for the first instance, or None when a
/// previous instance already holds the mutex (after signaling it).
fn single_instance_guard() -> Option<mpsc::Receiver<()>> {
    use windows::core::w;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, SetEvent, EVENT_ALL_ACCESS, EVENT_MODIFY_STATE,
    };
    unsafe {
        // If the show-event already exists, another instance is up: wake it.
        if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, w!("VstApplierRs.ShowWindow")) {
            let _ = SetEvent(ev);
            let _ = CloseHandle(ev);
            return None;
        }
        // First instance: create mutex (leaked for process lifetime) + event.
        // NOTE: HANDLE is Copy, so mem::forget would be a no-op; Box::leak
        // keeps the handle open until process exit instead.
        let mutex = CreateMutexW(None, false, w!("VstApplierRs.SingleInstance")).ok();
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
                    if tx.send(()).is_err() {
                        break;
                    }
                }
            }
        });
        Some(rx)
    }
}

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
        let settings = load_settings();
        let auto_start = settings.AutoStartMicrophone;
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default();
        let plugin_folder = if session.PluginFolder.is_empty() {
            default_plugin_folder(&exe_dir).to_string_lossy().to_string()
        } else {
            session.PluginFolder.clone()
        };
        let inputs = input_devices();
        let outputs = output_devices();
        let sel_input = pick_saved(&inputs, &session.InputDeviceId, &session.InputDeviceName);
        let mut sel_output = pick_saved(&outputs, &session.OutputDeviceId, &session.OutputDeviceName);
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
            sel_block: if ALLOWED_BLOCKS.contains(&session.BufferSize) {
                session.BufferSize
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
            chain: session
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
                let want = session.ActiveProfile.clone();
                if want.is_empty() || !profile_names().contains(&want) {
                    String::new()
                } else {
                    want
                }
            },
            new_profile_name: String::new(),
            show_new_profile: false,
            cable_msg: String::new(),
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
        app._tray = Some(tray::Tray::new());
        app.refresh_cable_status();
        // Restore per-plugin states into pending slots via session re-load on start.
        app.pending_states = session.Plugins.iter().map(|p| p.State.clone()).collect();
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

    fn capture_profile(&self, name: &str) -> VoiceSetupProfile {
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
                    State: self.pending_states.get(i).and_then(|s| s.clone()),
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
fn route_row(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(396.0, 30.0),
        egui::Layout::left_to_right(egui::Align::Center),
        f,
    );
}

/// Fixed label column so all route selectors start at the same x.
fn route_lab(ui: &mut egui::Ui, text: &str) {
    ui.scope(|ui| {
        ui.set_min_width(118.0);
        ui.label(text);
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

/// LED-segment level row: fixed label, segmented bar, 1-decimal readout.
fn level_row(ui: &mut egui::Ui, label: &str, level: f32) {
    ui.horizontal(|ui| {
        let (lr, _) = ui.allocate_exact_size(egui::vec2(60.0, 18.0), egui::Sense::hover());
        ui.painter().text(
            lr.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(14.0),
            egui::Color32::from_rgb(214, 228, 245),
        );
        let db = if level <= 0.0 {
            "-∞ dB".to_string()
        } else {
            format!("{:+.1} dB", 20.0 * level.log10())
        };
        let mw = (ui.available_width() - 156.0).max(80.0);
        let (mr, _) = ui.allocate_exact_size(egui::vec2(mw, 16.0), egui::Sense::hover());
        // Track bed so the segments read as one meter, not floating dashes.
        ui.painter().rect_filled(
            mr.expand2(egui::vec2(5.0, 4.0)),
            egui::CornerRadius::same(4),
            egui::Color32::from_rgb(28, 36, 50),
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
                    egui::Color32::from_rgb(52, 211, 153)
                } else if f < 0.85 {
                    egui::Color32::from_rgb(250, 204, 21)
                } else {
                    egui::Color32::from_rgb(248, 113, 113)
                }
            } else {
                egui::Color32::from_rgb(36, 46, 62)
            };
            ui.painter()
                .rect_filled(r, egui::CornerRadius::same(2), c);
        }
        let (dr, _) = ui.allocate_exact_size(egui::vec2(84.0, 18.0), egui::Sense::hover());
        ui.painter().text(
            dr.right_center(),
            egui::Align2::RIGHT_CENTER,
            db,
            egui::FontId::monospace(13.5),
            egui::Color32::WHITE,
        );
    });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Second-instance wake requests.
        while self.show_rx.try_recv().is_ok() {
            self.visible = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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
                    egui::Color32::from_rgb(52, 211, 153)
                } else {
                    egui::Color32::from_gray(110)
                };
                icons::logo(ui);
                ui.label(
                    egui::RichText::new("VST-Applier")
                        .strong()
                        .size(20.0)
                        .color(egui::Color32::WHITE),
                );
                let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, dot);
                ui.label(
                    egui::RichText::new(if running { "RUNNING" } else { "IDLE" })
                        .strong()
                        .color(dot),
                );
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
                    let del_resp = icons::icon_button(
                        ui,
                        icons::Icon::Trash,
                        "Delete",
                        egui::vec2(112.0, 34.0),
                    );
                    if del_resp.clicked()
                    {
                        if !self.sel_profile.is_empty() {
                            let name = self.sel_profile.clone();
                            let _ = delete_profile(&name);
                            self.profiles = profile_names();
                            self.sel_profile.clear();
                            self.status = format!("deleted '{name}'");
                        } else {
                            self.status = "Delete: no profile selected".into();
                        }
                    }
                    let save_resp = icons::icon_button(
                        ui,
                        icons::Icon::Disk,
                        "Save",
                        egui::vec2(104.0, 34.0),
                    );
                    if save_resp.clicked()
                    {
                        // Never write ".json": saving requires a name.
                        if self.sel_profile.is_empty() {
                            self.status = "Save: no profile selected — use New first".into();
                        } else {
                            let p = self.capture_profile(&self.sel_profile);
                            match save_profile(&p) {
                                Ok(()) => self.status = format!("saved '{}'", p.Name),
                                Err(e) => self.status = format!("save failed: {e}"),
                            }
                            self.profiles = profile_names();
                        }
                    }
                    let new_resp = icons::icon_button(
                        ui,
                        icons::Icon::Plus,
                        "New",
                        egui::vec2(96.0, 34.0),
                    );
                    if new_resp.clicked()
                    {
                        self.show_new_profile = true;
                    }
                    let prev = self.sel_profile.clone();
                    // allocate_ui_with_layout mis-positions in this flow
                    // (measured +8px low), so pin an exact rect and hand
                    // a child Ui over it: position becomes deterministic.
                    let (combo_rect, _) =
                        ui.allocate_exact_size(egui::vec2(200.0, 34.0), egui::Sense::hover());
                    let mut combo_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(combo_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    // ComboBox paints content-sized (unlike Button, which
                    // fills its rect): inflate padding to exactly 34.
                    combo_ui.spacing_mut().button_padding.y = 8.0;
                    let _combo_resp = egui::ComboBox::from_id_salt("profile")
                        .width(200.0)
                        .height(300.0)
                        .selected_text(if self.sel_profile.is_empty() {
                            "(none)".to_string()
                        } else {
                            self.sel_profile.clone()
                        })
                        .show_ui(&mut combo_ui, |ui| {
                            for name in self.profiles.clone() {
                                ui.selectable_value(
                                    &mut self.sel_profile,
                                    name.clone(),
                                    name,
                                );
                            }
                        });
                    if self.sel_profile != prev && !self.sel_profile.is_empty() {
                        let name = self.sel_profile.clone();
                        self.apply_profile(&name);
                    }
                    // Own 34px slot: a bare label centers on the
                    // placement-time row height, which drifts.
                    let _prof_slot = ui.allocate_ui_with_layout(
                        egui::vec2(58.0, 34.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label("Profile:");
                        },
                    );
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
            .resizable(false)
            .default_width(460.0)
            .max_width(500.0)
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
                // Independent scroll: keeps every control reachable on
                // short (1080p) windows without disturbing the center.
                // Capped to leave room for the bottom-pinned Settings:
                // an uncapped ScrollArea would fill the whole panel and
                // push Settings off-screen.
                egui::ScrollArea::vertical()
                    .id_salt("left")
                    .max_height((panel_h - 56.0).max(200.0))
                    .show(ui, |ui| {
                card2(
                    ui,
                    icons::Icon::Wave,
                    "Audio Route",
                    Some("Select your input, output and buffer settings"),
                    None::<fn(&mut egui::Ui)>,
                    |ui| {
                    // Inline rows (fixed widths — never `available_width`
                    // inside a SidePanel): icon + label column + selector
                    // column share one baseline; selectors align.
                    route_row(ui, |ui| {
                        icons::icon(ui, icons::Icon::Mic, icons::MIST);
                        route_lab(ui, "Input device");
                        egui::ComboBox::from_id_salt("in_sel")
                        .width(234.0)
                        .height(300.0)
                        .selected_text(if in_name.is_empty() {
                            "(select...)".to_string()
                        } else {
                            elide_middle(&in_name)
                        })
                        .show_ui(ui, |ui| {
                            for (id, name) in self.inputs.clone() {
                                ui.selectable_value(&mut self.sel_input, id.clone(), name);
                            }
                        })
                        .response
                        .on_hover_text(&in_name);
                    });
                    route_row(ui, |ui| {
                        icons::icon(ui, icons::Icon::Headset, icons::MIST);
                        route_lab(ui, "Output device");
                        egui::ComboBox::from_id_salt("out_sel")
                        .width(234.0)
                        .height(300.0)
                        .selected_text(if out_name.is_empty() {
                            "(select...)".to_string()
                        } else {
                            elide_middle(&out_name)
                        })
                        .show_ui(ui, |ui| {
                            for (id, name) in self.outputs.clone() {
                                ui.selectable_value(&mut self.sel_output, id.clone(), name);
                            }
                        })
                        .response
                        .on_hover_text(&out_name);
                    });
                    route_row(ui, |ui| {
                        icons::icon(ui, icons::Icon::Chip, icons::MIST);
                        route_lab(ui, "Buffer (frames)");
                        egui::ComboBox::from_id_salt("blk_sel")
                        .width(234.0)
                        .height(300.0)
                        .selected_text(format!("{}", self.sel_block))
                        .show_ui(ui, |ui| {
                            for &b in ALLOWED_BLOCKS {
                                ui.selectable_value(&mut self.sel_block, b, format!("{b}"));
                            }
                        });
                    });
                    ui.add_space(6.0);
                    if running {
                        if ui
                            .add_sized(
                                [408.0, 40.0],
                                egui::Button::new(
                                    egui::RichText::new("■ Stop")
                                        .strong()
                                        .size(17.0)
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(egui::Color32::from_rgb(190, 40, 70)),
                            )
                            .clicked()
                        {
                            self.stop_route();
                            self.status = "Stopped".into();
                            self.status_diag = "Engine idle.".into();
                        }
                    } else {
                        let ready =
                            !self.sel_input.is_empty() && !self.sel_output.is_empty();
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(ready, |ui| {
                                if ui
                                    .add_sized(
                                        [228.0, 40.0],
                                        egui::Button::new(
                                            egui::RichText::new("▶ Start")
                                                .strong()
                                                .size(17.0)
                                                .color(egui::Color32::WHITE),
                                        )
                                        .fill(egui::Color32::from_rgb(229, 72, 77)),
                                    )
                                    .on_disabled_hover_text(
                                        "Select an input and an output first",
                                    )
                                    .clicked()
                                {
                                    self.try_start();
                                }
                            });
                            if ui
                                .add_sized([174.0, 40.0], egui::Button::new("↻ Refresh"))
                                .clicked()
                            {
                                self.refresh_devices();
                            }
                        });
                    }
                    },
                );
                let cable_status = self.cable_msg.clone();
                card2(
                    ui,
                    icons::Icon::Link,
                    "Virtual cable",
                    None,
                    Some(|ui: &mut egui::Ui| {
                        egui::Frame::default()
                            .fill(egui::Color32::from_rgb(13, 60, 50))
                            .corner_radius(egui::CornerRadius::same(10))
                            .inner_margin(egui::Margin::symmetric(8, 3))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(
                                        egui::vec2(8.0, 8.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        r.center(),
                                        4.0,
                                        egui::Color32::from_rgb(52, 211, 153),
                                    );
                                    ui.label(
                                        egui::RichText::new(&cable_status)
                                            .size(12.0)
                                            .color(egui::Color32::from_rgb(110, 231, 183)),
                                    );
                                });
                            });
                    }),
                    |ui| {
                    egui::Grid::new("cable_grid")
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Input:");
                            ui.label(&in_name);
                            ui.end_row();
                            ui.label("Output:");
                            ui.label(&out_name);
                            ui.end_row();
                        });
                    },
                );
                card2(
                    ui,
                    icons::Icon::Bars,
                    "Levels",
                    None,
                    None::<fn(&mut egui::Ui)>,
                    |ui| {
                    level_row(ui, "Input", in_peak);
                    ui.add_space(4.0);
                    level_row(ui, "Output", out_peak);
                    },
                );
                // Live engine telemetry (real measurements, same source
                // as the status bar): fills the sidebar deliberately.
                card2(
                    ui,
                    icons::Icon::Gauge,
                    "Engine",
                    None,
                    None::<fn(&mut egui::Ui)>,
                    |ui| {
                    if let Some(r) = self.route.as_ref() {
                        let d = r.diagnostics();
                        let (xr, _trim) = r.dropouts();
                        // Two compact rows: fits the column without
                        // forcing the sidebar to scroll.
                        egui::Grid::new("engine_grid")
                            .num_columns(4)
                            .spacing([6.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("State:");
                                ui.label(
                                    egui::RichText::new("Running").color(
                                        egui::Color32::from_rgb(52, 211, 153),
                                    ),
                                );
                                ui.label("Latency:");
                                ui.label(format!("~{:.0} ms", d.buffered_ms));
                                ui.end_row();
                                ui.label("Block:");
                                ui.label(format!("{}fr", d.plugin_block_size));
                                ui.label("Dropouts:");
                                ui.label(format!("{xr}"));
                                ui.end_row();
                            });
                    } else {
                        ui.label(
                            egui::RichText::new("Engine idle — press Start.").weak(),
                        );
                    }
                    },
                );
                }); // end left scroll
                // Deliberate anchor: a rule above the pinned Settings so
                // the bottom-dock reads as a section, not a stray card.
                ui.separator();
                // Pin Settings to the sidebar bottom: absorb leftover
                // height above it (max 0 on short windows, where the
                // scroll above already keeps everything reachable).
                ui.add_space((panel_h - (ui.next_widget_position().y - top) - 56.0).max(0.0));
                let sresp = egui::Frame::default()
                    .fill(egui::Color32::from_rgb(20, 26, 36))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgb(48, 60, 82),
                    ))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::same(10))
                    .outer_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: 4,
                        bottom: 4,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icons::icon(ui, icons::Icon::Gear, icons::MIST);
                            ui.label(
                                egui::RichText::new("Settings")
                                    .strong()
                                    .size(14.0)
                                    .color(egui::Color32::WHITE),
                            );
                            let gap = (ui.available_width() - 24.0).max(0.0);
                            ui.add_space(gap);
                            icons::icon(
                                ui,
                                icons::Icon::ChevR,
                                egui::Color32::from_rgb(148, 163, 184),
                            );
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

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("center").show(ui, |ui| {
            // Active chain FIRST — this is what you watch while routing.
            let mut mv: Option<(usize, isize)> = None;
            let mut rm: Option<usize> = None;
            let mut ed: Option<usize> = None;
            let mut changed = false;
            let clear_all = Cell::new(false);
            let load_order = Cell::new(false);
            let chain_sub = format!(
                "VST active: {}/{} enabled, block {}, {}",
                self.chain.iter().filter(|c| c.enabled).count(),
                self.chain.len(),
                self.sel_block,
                if self.route.is_some() { "running" } else { "stopped" },
            );
            card2(
                ui,
                icons::Icon::Layers,
                "Plugin chain",
                Some(chain_sub.as_str()),
                Some(|ui: &mut egui::Ui| {
                    if icons::icon_button(
                        ui,
                        icons::Icon::SwapV,
                        "Load order",
                        egui::vec2(140.0, 32.0),
                    )
                    .on_hover_text("Reset chain to the selected profile's order")
                    .clicked()
                    {
                        load_order.set(true);
                    }
                    if icons::icon_button(
                        ui,
                        icons::Icon::Trash,
                        "Clear chain",
                        egui::vec2(140.0, 32.0),
                    )
                    .on_hover_text("Remove all plugins")
                    .clicked()
                    {
                        clear_all.set(true);
                    }
                }),
                |ui| {
                        for (i, item) in self.chain.iter_mut().enumerate() {
                            egui::Frame::default()
                                .fill(egui::Color32::from_rgb(24, 30, 42))
                                .corner_radius(egui::CornerRadius::same(6))
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    // Fixed-height strip with vertical
                                    // centering (horizontal_centered would
                                    // grab all available height).
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(ui.available_width(), 34.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                // Order number.
                                egui::Frame::default()
                                    .fill(egui::Color32::from_rgb(36, 52, 80))
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(8, 3))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(format!("{}", i + 1)).strong(),
                                        );
                                    });
                                // Reference enable/disable checkbox.
                                if icons::checkbox_red(ui, &mut item.enabled) {
                                    changed = true;
                                }
                                // Format pill.
                                let (badge, pill_bg, pill_fg) = match item.format {
                                    PluginFormat::Vst3 => (
                                        "VST3",
                                        (13, 84, 76),
                                        (94, 234, 212),
                                    ),
                                    PluginFormat::Vst2 => (
                                        "VST2",
                                        (74, 53, 10),
                                        (252, 211, 77),
                                    ),
                                };
                                egui::Frame::default()
                                    .fill(egui::Color32::from_rgb(pill_bg.0, pill_bg.1, pill_bg.2))
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(8, 4))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(badge)
                                                .size(12.0)
                                                .strong()
                                                .color(egui::Color32::from_rgb(
                                                    pill_fg.0, pill_fg.1, pill_fg.2,
                                                )),
                                        );
                                    });
                                ui.label(egui::RichText::new(&item.name).strong().size(16.0));
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
                                            icons::icon_button(
                                                ui,
                                                icons::Icon::Sliders,
                                                ed_txt,
                                                egui::vec2(84.0, 28.0),
                                            )
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
                        if let Some((i, d)) = mv {
                            let j = (i as isize + d).clamp(0, self.chain.len() as isize - 1) as usize;
                            if j != i {
                                // Harvest first: stop_route() overwrites
                                // pending in route order, so the swap must
                                // come after.
                                let was_running = self.route.is_some();
                                self.stop_route();
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
                            self.chain.clear();
                            self.pending_states.clear();
                            self.status = "chain cleared".into();
                            if was_running {
                                self.try_start();
                            }
                        }
                        if load_order.get() {
                            self.load_profile_order();
                        }
                        if self.chain.is_empty() {
                            ui.weak("chain is empty — add plugins below");
                        }
                    },
                ); // end Plugin chain card
                // Available panel: manual frame so the heading row (search +
                // Scan) and the table body can both use app state directly.
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(20, 26, 36))
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgb(48, 60, 82),
                    ))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::same(14))
                    .outer_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: 6,
                        bottom: 6,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icons::icon(ui, icons::Icon::Puzzle, icons::CORAL);
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new("Available VST plugins")
                                        .strong()
                                        .size(17.0)
                                        .color(egui::Color32::WHITE),
                                );
                                ui.label(
                                    egui::RichText::new("Scan and add plugins to your chain")
                                        .size(13.0)
                                        .color(egui::Color32::from_rgb(148, 163, 184)),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let scan_resp = ui
                                        .add_enabled_ui(!self.scanning, |ui| {
                                            icons::icon_button(
                                                ui,
                                                icons::Icon::Folder,
                                                if self.scanning {
                                                    "…"
                                                } else {
                                                    "Scan"
                                                },
                                                egui::vec2(110.0, 32.0),
                                            )
                                        })
                                        .inner
                                        .on_hover_text("Rescan the plugin folder");
                                    if scan_resp.clicked()
                                    {
                                        self.start_scan();
                                    }
                                    ui.add_sized(
                                        [260.0, 32.0],
                                        egui::TextEdit::singleline(&mut self.plugin_search)
                                            .hint_text("Search plugins..."),
                                    );
                                },
                            );
                        });
                        ui.add_space(6.0);
                        let q = self.plugin_search.to_lowercase();
                        // Column labels aligned to the fixed row slots below.
                        ui.horizontal(|ui| {
                            // 7px inset mirrors the badge frame's left
                            // padding so "Type" sits over the pills.
                            ui.add_space(7.0);
                            ui.scope(|ui| {
                                ui.set_min_width(73.0);
                                ui.label(
                                    egui::RichText::new("Type")
                                        .size(12.0)
                                        .color(egui::Color32::from_rgb(120, 130, 145)),
                                );
                            });
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("Name")
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(120, 130, 145)),
                            );
                        });
                        // Borderless list body: rows on the card surface
                        // with dividers only (no spreadsheet box).
                        egui::Frame::default()
                            .inner_margin(egui::Margin::symmetric(0, 2))
                            .show(ui, |ui| {
                                // Height fits the visible rows (no void
                                // under short lists), capped so the folder
                                // footer stays on screen.
                                let visible = self
                                    .found
                                    .iter()
                                    .filter(|c| {
                                        q.is_empty() || c.name.to_lowercase().contains(&q)
                                    })
                                    .count();
                                // One 39px pitch per row (30px content +
                                // 9px painted divider, zero item gap). The
                                // cap ends a few px inside a divider zone
                                // so fractional layout slack never shows
                                // a row sliver.
                                let list_h =
                                    (visible as f32 * 39.0).clamp(140.0, 269.0);
                                egui::ScrollArea::vertical()
                                    .id_salt("avail_list")
                                    .max_height(list_h)
                                    .show(ui, |ui| {
                                        // Zero vertical item gap inside the
                                        // list: pitch is exactly 30px row +
                                        // 9px divider, matching list_h.
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        let mut first = true;
                                        for (k, c) in
                                            self.found.clone().into_iter().enumerate()
                                        {
                                            if !q.is_empty()
                                                && !c.name.to_lowercase().contains(&q)
                                            {
                                                continue;
                                            }
                                            if !first {
                                                // Fixed 9px divider (a line
                                                // centered in 9px) — exact
                                                // pitch, unlike separator().
                                                let (dr, _) = ui.allocate_exact_size(
                                                    egui::vec2(ui.available_width(), 9.0),
                                                    egui::Sense::hover(),
                                                );
                                                ui.painter().line_segment(
                                                    [dr.left_center(), dr.right_center()],
                                                    egui::Stroke::new(
                                                        1.0_f32,
                                                        egui::Color32::from_rgb(48, 60, 82),
                                                    ),
                                                );
                                            }
                                            first = false;
                                            ui.push_id(("avail", k), |ui| {
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(ui.available_width(), 30.0),
                                                    egui::Layout::left_to_right(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        let (badge, bg, fg) = match c.format {
                                                            app_state::CandidateFormat::Vst3 => {
                                                                (
                                                                    "VST3",
                                                                    (13, 84, 76),
                                                                    (94, 234, 212),
                                                                )
                                                            }
                                                            app_state::CandidateFormat::Vst2 => {
                                                                (
                                                                    "VST2",
                                                                    (74, 53, 10),
                                                                    (252, 211, 77),
                                                                )
                                                            }
                                                        };
                                                        // Fixed Type slot (constant — no
                                                        // width feedback) so names align.
                                                        ui.scope(|ui| {
                                                            ui.set_min_width(80.0);
                                                            egui::Frame::default()
                                                                .fill(egui::Color32::from_rgb(
                                                                    bg.0, bg.1, bg.2,
                                                                ))
                                                                .corner_radius(
                                                                    egui::CornerRadius::same(4),
                                                                )
                                                                .inner_margin(
                                                                    egui::Margin::symmetric(7, 3),
                                                                )
                                                                .show(ui, |ui| {
                                                                    ui.label(
                                                                        egui::RichText::new(badge)
                                                                            .size(12.0)
                                                                            .strong()
                                                                            .color(
                                                                                egui::Color32::from_rgb(
                                                                                    fg.0, fg.1, fg.2,
                                                                                ),
                                                                            ),
                                                                    );
                                                                });
                                                        });
                                                        ui.add_space(4.0);
                                                        ui.label(
                                                            egui::RichText::new(&c.name)
                                                                .size(14.0)
                                                                .color(egui::Color32::from_rgb(
                                                                    214, 222, 235,
                                                                )),
                                                        );
                                                        ui.with_layout(
                                                            egui::Layout::right_to_left(
                                                                egui::Align::Center,
                                                            ),
                                                            |ui| {
                                                                if ui
                                                                    .add_sized(
                                                                        [64.0, 26.0],
                                                                        egui::Button::new(
                                                                            egui::RichText::new(
                                                                                "Add",
                                                                            )
                                                                            .size(12.5),
                                                                        ),
                                                                    )
                                                                    .clicked()
                                                                {
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
                                    });
                            }); // end bordered table
                    }); // end Available panel
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Plugin folder:");
                let fw = (ui.available_width() - 118.0).max(120.0);
                ui.add_sized(
                    [fw, 32.0],
                    egui::TextEdit::singleline(&mut self.plugin_folder),
                );
                let fscan = ui
                    .add_enabled_ui(!self.scanning, |ui| {
                        icons::icon_button(
                            ui,
                            icons::Icon::Folder,
                            if self.scanning { "…" } else { "Scan" },
                            egui::vec2(110.0, 32.0),
                        )
                    })
                    .inner
                    .on_hover_text("Rescan using this folder path");
                if fscan.clicked()
                {
                    self.start_scan();
                }
                ui.add_space(6.0);
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.monospace(&self.status)
                    .on_hover_text(&self.status_diag);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("v{}-rs", update::APP_VERSION)).small().weak());
                });
            });
        });

        // Borderless-frame edge grips (above panels, below dialogs).
        frame::edge_zones(ctx);

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
                            self.sel_profile = name.clone();
                            let p = self.capture_profile(&name);
                            let _ = save_profile(&p);
                            self.profiles = profile_names();
                            self.status = format!("created '{name}'");
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

