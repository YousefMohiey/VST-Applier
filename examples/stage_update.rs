//! One-shot proof: check + download + extract the live release asset.
#[path = "../src/update.rs"]
mod update;

fn main() {
    // Version compare already proven ("current (v0.2.0)"); here we prove
    // download + extract against the real v0.2.0 asset bytes.
    let rel = update::ReleaseInfo {
        tag: "v0.2.0".into(),
        name: "VST-Applier RS v0.2.0".into(),
        zip_url: "https://github.com/YousefMohiey/VST-Applier/releases/download/v0.2.0/VST-Applier-rs-v0.2.0-win64.zip".into(),
        zip_name: "VST-Applier-rs-v0.2.0-win64.zip".into(),
    };
    let stage = update::download_and_stage(&rel).expect("stage");
    for f in ["vst-applier.exe", "Vst2HostNative.dll", "Vst3HostNative.dll"] {
        assert!(stage.join(f).exists(), "missing {f}");
    }
    println!("STAGE OK: {}", stage.display());
}
