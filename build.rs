//! Windows-only: embed the app icon (and version info) into the exe so
//! Explorer, the taskbar, and Alt-Tab show the redesigned mic artwork.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/app.ico");
    if let Err(e) = res.compile() {
        eprintln!("winres failed (exe will have no icon): {e}");
    }
}
