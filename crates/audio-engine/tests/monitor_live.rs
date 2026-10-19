//! Live integration test for the "hear yourself" monitor.
//!
//! Uses `VA_FAKE_CAPTURE=1` so no microphone is opened and no acoustic
//! loop can form; the route renders a quiet 440 Hz sine into the chosen
//! output (CABLE Input when present). Device-dependent by nature: on a
//! host with fewer than two render endpoints the test self-skips.

use std::time::{Duration, Instant};

fn pump(route: &mut audio_engine::RunningRoute, ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        route.poll_preload();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn monitor_tees_processed_audio_without_touching_route() {
    // SAFETY: this test binary holds a single test; nothing concurrent
    // reads the environment here.
    unsafe { std::env::set_var("VA_FAKE_CAPTURE", "1") };

    let outs = audio_engine::output_devices();
    if outs.len() < 2 {
        eprintln!(
            "SKIP monitor_tees_processed_audio_without_touching_route: {} output device(s)",
            outs.len()
        );
        return;
    }
    let route_dev = outs
        .iter()
        .find(|(_, n)| n.contains("CABLE Input"))
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| audio_engine::default_output_device_name().unwrap_or_default());
    let capture = audio_engine::input_devices()
        .first()
        .map(|(id, _)| id.clone())
        .unwrap_or_default();
    let safe = |n: &str| audio_engine::monitor::device_problem(n, &route_dev, &capture).is_none();
    let monitor_dev = {
        let def = audio_engine::default_output_device_name().unwrap_or_default();
        if !def.is_empty() && safe(&def) {
            def
        } else {
            outs.iter()
                .map(|(_, n)| n.clone())
                .find(|n| safe(n))
                .unwrap_or_default()
        }
    };
    assert!(!route_dev.is_empty(), "route output");
    assert!(
        !monitor_dev.is_empty(),
        "no safe monitor endpoint (route={route_dev})"
    );
    eprintln!("route='{route_dev}' monitor='{monitor_dev}'");

    let mut route = audio_engine::start(&capture, &route_dev, vec![], 512, None)
        .expect("route start with fake capture");
    pump(&mut route, 2000);
    let (xr0, trim0) = route.dropouts();
    let out0 = route.output_frames();
    assert!(out0 > 0, "route output stream never started");

    // Safety: monitoring into the route output is refused, route alive.
    assert!(
        route.enable_monitor(&route_dev).is_err(),
        "monitor must refuse the route output"
    );
    // A missing device is a normal best-effort failure, not a route event.
    assert!(
        route
            .enable_monitor("__vst_applier_no_such_device__")
            .is_err(),
        "monitor must fail cleanly on a missing device"
    );
    pump(&mut route, 300);
    assert!(route.monitor_device().is_none());
    assert!(
        route.output_frames() > out0,
        "route stalled on monitor failure"
    );

    // Real monitor device: stream comes up with the exact status line.
    route
        .enable_monitor(&monitor_dev)
        .expect("monitor enable on a second device");
    let want = format!("monitor: stream active on {monitor_dev}");
    assert_eq!(route.monitor_status().as_deref(), Some(want.as_str()));

    pump(&mut route, 2000);
    let (xr1, trim1) = route.dropouts();
    let out1 = route.output_frames();
    let (frames, _underruns, _drops, peak) =
        route.monitor_counters().expect("monitor counters while on");

    assert!(frames > 0, "monitor stream rendered no frames");
    assert!(
        peak > 0.005,
        "monitor peak {peak} — tee not reaching the stream"
    );
    // The whole point: monitor work must never disturb the cable feed.
    assert_eq!(xr1, xr0, "route dropouts increased with monitor on");
    assert_eq!(trim1, trim0, "route trims increased with monitor on");
    assert!(out1 > out0, "route output stalled with monitor on");

    // Off again: consumer returned, route keeps rendering.
    route.disable_monitor();
    assert!(route.monitor_device().is_none());
    assert!(route.monitor_counters().is_none());
    pump(&mut route, 500);
    let (xr2, trim2) = route.dropouts();
    assert!(
        route.output_frames() > out1,
        "route stalled after monitor off"
    );
    assert_eq!(xr2, xr1, "route dropouts increased after monitor off");
    assert_eq!(trim2, trim1, "route trims increased after monitor off");

    // Re-enable works (consumer was reclaimed, not lost).
    route
        .enable_monitor(&monitor_dev)
        .expect("monitor re-enable after disable");
    pump(&mut route, 500);
    let (frames2, ..) = route
        .monitor_counters()
        .expect("monitor counters after re-enable");
    assert!(frames2 > 0, "monitor produced no frames after re-enable");
    route.disable_monitor();
}
