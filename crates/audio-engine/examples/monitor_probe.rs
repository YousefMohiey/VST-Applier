//! Live probe for the "hear yourself" monitor.
//!
//! Runs a REAL route with `VA_FAKE_CAPTURE=1` (quiet 440 Hz sine, no mic,
//! no acoustic loop), enables the monitor on a second output device,
//! checks that the monitor stream actually renders audio and that the
//! route's dropout counters do not move, then exits.
//!
//! Device choice:
//!   VA_ROUTE_DEVICE   route output (default: a "CABLE Input" endpoint,
//!                     else the Windows default output)
//!   VA_MONITOR_DEVICE monitor device (default: default output if it is
//!                     not the route output, else the first safe output)
//!
//! Exit 0 = PASS, 1 = FAIL. All log lines are printed to stderr.

use std::time::{Duration, Instant};

fn main() {
    // SAFETY: single-threaded probe process; set before any audio thread.
    unsafe { std::env::set_var("VA_FAKE_CAPTURE", "1") };

    let outs = audio_engine::output_devices();
    if outs.len() < 2 {
        eprintln!("SKIP: need 2+ output devices, found {}", outs.len());
        std::process::exit(0);
    }
    println!(
        "outputs: {}",
        outs.iter()
            .map(|(_, n)| n.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    );

    let route_dev = std::env::var("VA_ROUTE_DEVICE").ok().unwrap_or_else(|| {
        outs.iter()
            .find(|(_, n)| n.contains("CABLE Input"))
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| audio_engine::default_output_device_name().unwrap_or_default())
    });
    if route_dev.is_empty() {
        eprintln!("FAIL: no route output device");
        std::process::exit(1);
    }

    let capture = audio_engine::input_devices()
        .first()
        .map(|(id, _)| id.clone())
        .unwrap_or_default();
    let safe = |n: &str| audio_engine::monitor::device_problem(n, &route_dev, &capture).is_none();

    let monitor = std::env::var("VA_MONITOR_DEVICE").ok().unwrap_or_else(|| {
        let def = audio_engine::default_output_device_name().unwrap_or_default();
        if !def.is_empty() && safe(&def) {
            def
        } else {
            outs.iter()
                .map(|(_, n)| n.clone())
                .find(|n| safe(n))
                .unwrap_or_default()
        }
    });
    if monitor.is_empty() {
        eprintln!("FAIL: no safe monitor device (route='{route_dev}')");
        std::process::exit(1);
    }

    println!("route output: {route_dev}");
    println!("monitor device: {monitor}");

    let mut route = match audio_engine::start(&capture, &route_dev, vec![], 512, None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("FAIL: route start: {e}");
            std::process::exit(1);
        }
    };
    pump(&mut route, 2500, Some(Duration::from_secs(2)));
    let (xr0, trim0) = route.dropouts();
    let out0 = route.output_frames();
    if out0 == 0 {
        eprintln!("FAIL: route produced no output frames (output never started)");
        std::process::exit(1);
    }

    // 1) Safety: the route's own output must be refused with the exact
    //    `monitor: disabled (<reason>)` log line, route untouched.
    match route.enable_monitor(&route_dev) {
        Ok(()) => {
            eprintln!("FAIL: monitor accepted the route output (must refuse)");
            std::process::exit(1);
        }
        Err(why) => println!("PASS refusal: monitor: disabled ({why})"),
    }

    // 1b) Missing device: best-effort failure, route must keep running.
    match route.enable_monitor("__vst_applier_no_such_device__") {
        Ok(()) => {
            eprintln!("FAIL: monitor accepted a missing device");
            std::process::exit(1);
        }
        Err(why) => println!("PASS missing device: monitor: disabled ({why})"),
    }
    pump(&mut route, 400, None);
    if route.output_frames() <= out0 {
        eprintln!("FAIL: route stalled on a monitor failure");
        std::process::exit(1);
    }

    // 2) Real monitor stream on a second device.
    if let Err(why) = route.enable_monitor(&monitor) {
        eprintln!("FAIL: monitor enable failed: {why}");
        std::process::exit(1);
    }
    let status = route.monitor_status().unwrap_or_default();
    println!("status: {status}");
    if status != format!("monitor: stream active on {monitor}") {
        eprintln!("FAIL: unexpected monitor status '{status}'");
        std::process::exit(1);
    }

    pump(&mut route, 2500, Some(Duration::from_secs(2)));
    let (frames, underruns, drops, peak) = route.monitor_counters().unwrap_or((0, 0, 0, 0.0));
    let (xr1, trim1) = route.dropouts();
    let out1 = route.output_frames();

    // 3) Monitor OFF again: route must keep running.
    route.disable_monitor();
    pump(&mut route, 600, None);
    let (xr2, trim2) = route.dropouts();
    let out2 = route.output_frames();

    println!(
        "monitor counters: frames={frames} underruns={underruns} drops={drops} peak={peak:.4}"
    );
    println!("route dropouts: before=({xr0},{trim0}) during=({xr1},{trim1}) after=({xr2},{trim2})");
    println!("route output frames: before={out0} during={out1} after={out2}");

    let mut fail = Vec::new();
    if frames == 0 {
        fail.push("monitor rendered 0 frames");
    }
    if peak <= 0.005 {
        fail.push("monitor peak ~0 (tee not reaching the stream)");
    }
    if xr1 > xr0 {
        fail.push("route dropouts increased while monitor was on");
    }
    if trim1 > trim0 {
        fail.push("route trims increased while monitor was on");
    }
    if out1 <= out0 || out2 <= out1 {
        fail.push("route output stopped while monitor toggled");
    }
    if xr2 > xr1 {
        fail.push("route dropouts increased after monitor off");
    }

    if fail.is_empty() {
        println!("PASS monitor probe");
    } else {
        for f in fail {
            eprintln!("FAIL: {f}");
        }
        std::process::exit(1);
    }
}

/// Drive `poll_preload` (the UI's job in the app) for `ms`, breaking
/// early once `ready_after` has elapsed *and* output frames advance.
fn pump(route: &mut audio_engine::RunningRoute, ms: u64, ready_after: Option<Duration>) {
    let start = Instant::now();
    let mut last_frames = route.output_frames();
    let mut last_progress = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        route.poll_preload();
        let now = route.output_frames();
        if now != last_frames {
            last_frames = now;
            last_progress = Instant::now();
        }
        if let Some(after) = ready_after {
            if start.elapsed() >= after && last_progress.elapsed() < Duration::from_millis(150) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
