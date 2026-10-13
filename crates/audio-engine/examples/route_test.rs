fn main() {
    let inputs = audio_engine::input_devices();
    let outputs = audio_engine::output_devices();
    let input = inputs
        .iter()
        .find(|(_, n)| n.contains("CABLE Output"))
        .map(|(id, _)| id.clone())
        .expect("no CABLE Output input");
    let output = outputs
        .iter()
        .find(|(_, n)| n == "CABLE Input (VB-Audio Virtual Cable)")
        .map(|(id, _)| id.clone())
        .expect("no CABLE Input output");
    println!("routing '{input}' -> '{output}' (dry, 6s)");
    let mut route = audio_engine::start(&input, &output, vec![], 512).expect("start failed");
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        route.poll_preload();
        let d = route.diagnostics();
        println!(
            "{} | in_peak={:.3} out_peak={:.3}",
            route.status_line(),
            route.meters().input(),
            route.meters().output(),
        );
        let _ = d;
    }
}
