//! Headless proof that a saved profile persists everything: devices,
//! buffer, plugin folder, the full plugin chain, and each plugin's state
//! blob — plus the ActiveProfile round-trip the boot logic relies on.
//!
//! Run: cargo run -p app-state --example profile_roundtrip

use app_state::{
    delete_profile, load_profile, save_profile, VoiceSetupPluginEntry, VoiceSetupProfile,
};

fn entry(name: &str, state: Option<&str>) -> VoiceSetupPluginEntry {
    VoiceSetupPluginEntry {
        Name: name.into(),
        Path: format!("C:\\Program Files\\VST-Applier\\Plugins\\{name}\\{name}.vst3"),
        Format: "Vst3".into(),
        Enabled: true,
        State: state.map(|s| s.to_string()),
    }
}

fn main() {
    let name = "RoundtripProbe";
    let _ = delete_profile(name); // clean slate

    let original = VoiceSetupProfile {
        SchemaVersion: 1,
        Name: name.into(),
        ActiveProfile: name.into(),
        PluginFolder: "C:\\Program Files\\VST-Applier\\Plugins".into(),
        InputDeviceId: "in-id-123".into(),
        InputDeviceName: "Headset Microphone (Test)".into(),
        OutputDeviceId: "out-id-456".into(),
        OutputDeviceName: "CABLE Input (VB-Audio Virtual Cable)".into(),
        BufferSize: 1024,
        Plugins: vec![
            entry("rnnoise", Some("QUJDREVG")), // base64-ish blob
            entry("ReaGate", None),
            entry("TDR Nova", Some("R0hJSktM")),
        ],
    };

    save_profile(&original).expect("save");
    let back = load_profile(name).expect("load");

    let mut checks: Vec<(&str, bool)> = vec![
        ("SchemaVersion", back.SchemaVersion == original.SchemaVersion),
        ("Name", back.Name == original.Name),
        ("ActiveProfile", back.ActiveProfile == original.ActiveProfile),
        ("PluginFolder", back.PluginFolder == original.PluginFolder),
        ("InputDeviceId", back.InputDeviceId == original.InputDeviceId),
        ("InputDeviceName", back.InputDeviceName == original.InputDeviceName),
        ("OutputDeviceId", back.OutputDeviceId == original.OutputDeviceId),
        ("OutputDeviceName", back.OutputDeviceName == original.OutputDeviceName),
        ("BufferSize", back.BufferSize == original.BufferSize),
        ("Plugins.len", back.Plugins.len() == original.Plugins.len()),
    ];
    for (i, (a, b)) in back.Plugins.iter().zip(original.Plugins.iter()).enumerate() {
        checks.push((Box::leak(format!("p{i}.Name").into_boxed_str()), a.Name == b.Name));
        checks.push((Box::leak(format!("p{i}.Path").into_boxed_str()), a.Path == b.Path));
        checks.push((Box::leak(format!("p{i}.Format").into_boxed_str()), a.Format == b.Format));
        checks.push((Box::leak(format!("p{i}.Enabled").into_boxed_str()), a.Enabled == b.Enabled));
        checks.push((Box::leak(format!("p{i}.State").into_boxed_str()), a.State == b.State));
    }

    let mut failed = 0;
    for (label, ok) in &checks {
        if !ok {
            failed += 1;
            println!("FAIL  {label}");
        }
    }
    println!(
        "profile_roundtrip: {} checks, {} failed -> {}",
        checks.len(),
        failed,
        if failed == 0 { "ALL OK" } else { "BROKEN" }
    );
    let states: Vec<&Option<String>> = back.Plugins.iter().map(|p| &p.State).collect();
    println!("states read back: {states:?}");

    let _ = delete_profile(name);
    if failed > 0 {
        std::process::exit(1);
    }
}
