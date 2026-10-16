//! Dry-route soak: Maonocaster -> CABLE Input with an EMPTY chain (no
//! plugins), low-latency render. Bisect tool: if the full soak wedges
//! but this one doesn't, a plugin is implicated; if both wedge, the
//! route/ring/render path itself is.
//!
//!   VA_SOAK_SECS=300 cargo run --release -p audio-engine --example route_test
//!   VA_NO_LOWLAT=1 VA_SOAK_SECS=300 cargo run ...   (force cpal output)

fn main() {
    let inputs = audio_engine::input_devices();
    let outputs = audio_engine::output_devices();
    let input = inputs
        .iter()
        .find(|(_, n)| n.contains("Maonocaster"))
        .map(|(id, _)| id.clone())
        .expect("no Maonocaster input");
    let output = outputs
        .iter()
        .find(|(_, n)| n == "CABLE Input (VB-Audio Virtual Cable)")
        .map(|(id, _)| id.clone())
        .expect("no CABLE Input output");
    let secs: u64 = std::env::var("VA_SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    println!("dry route '{input}' -> '{output}' for {secs}s");
    let mut route = audio_engine::start(&input, &output, vec![], 512, None).expect("start failed");
    let t0 = std::time::Instant::now();
    let mut last = (0u64, std::time::Instant::now());
    while t0.elapsed().as_secs() < secs {
        std::thread::sleep(std::time::Duration::from_millis(2000));
        route.poll_preload();
        let s = route.status_line();
        // inb <n> inf <n> outf <n> — cheap scrape for the input-rate delta.
        let f = |tag: &str| -> u64 {
            s.split(tag)
                .nth(1)
                .and_then(|r| r.split_whitespace().next())
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        let inf = f(" inf ");
        let dt = last.1.elapsed().as_secs_f64();
        let in_rate = (inf - last.0) as f64 / dt;
        last = (inf, std::time::Instant::now());
        println!("[{}s] {s}", t0.elapsed().as_secs());
        if in_rate < 41_000.0 {
            println!("  !! capture delivery {in_rate:.0} frames/s — DEGRADED");
        }
    }
    let (xr, trim) = route.dropouts();
    println!("DRY DONE: dropouts={xr} trim={trim}");
    drop(route);
    println!("route dropped cleanly");
}
