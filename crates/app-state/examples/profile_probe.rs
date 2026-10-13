//! Profile store round-trip: save -> list -> load -> overwrite ->
//! delete, plus name validation. Uses a zz- name and cleans up.
use app_state::{
    delete_profile, is_valid_profile_name, load_profile, profile_names, save_profile,
    VoiceSetupPluginEntry, VoiceSetupProfile,
};

fn main() {
    assert!(!is_valid_profile_name(""));
    assert!(!is_valid_profile_name("a/b"));
    assert!(!is_valid_profile_name(&"x".repeat(49)));
    assert!(is_valid_profile_name("my voice 1"));
    println!("validation OK");

    let mk = |buf: u32, st: &str| VoiceSetupProfile {
        SchemaVersion: 1,
        Name: "zz-probe".into(),
        ActiveProfile: "zz-probe".into(),
        PluginFolder: "C:\\Plug".into(),
        InputDeviceId: "in-id".into(),
        InputDeviceName: "Mic".into(),
        OutputDeviceId: "out-id".into(),
        OutputDeviceName: "CABLE Input".into(),
        BufferSize: buf,
        Plugins: vec![VoiceSetupPluginEntry {
            Name: "reaeq".into(),
            Path: "C:\\reaeq.dll".into(),
            Format: "Vst2".into(),
            Enabled: true,
            State: Some(st.into()),
        }],
    };
    save_profile(&mk(512, "AAA=")).expect("save");
    assert!(profile_names().contains(&"zz-probe".to_string()), "listed");
    let p = load_profile("zz-probe").expect("load");
    assert_eq!(p.BufferSize, 512);
    assert_eq!(p.Plugins.len(), 1);
    assert_eq!(p.Plugins[0].State.as_deref(), Some("AAA="));
    println!("save/list/load OK");

    save_profile(&mk(256, "BBB=")).expect("overwrite");
    let p2 = load_profile("zz-probe").expect("reload");
    assert_eq!(p2.BufferSize, 256);
    assert_eq!(p2.Plugins[0].State.as_deref(), Some("BBB="));
    println!("overwrite OK");

    delete_profile("zz-probe").expect("delete");
    assert!(load_profile("zz-probe").is_none(), "gone");
    assert!(!profile_names().contains(&"zz-probe".to_string()), "unlisted");
    println!("delete OK");
    println!("PROFILE-STORE OK");
}
