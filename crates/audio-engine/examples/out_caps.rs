//! Probe: list output devices and their supported configs (scratch tool).
fn main() {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let want = std::env::args().nth(1).unwrap_or("CABLE".into());
    for d in host.output_devices().unwrap() {
        let name = d.name().unwrap_or_default();
        if !want.is_empty() && !name.contains(want.as_str()) {
            continue;
        }
        println!("== {name}");
        match d.supported_output_configs() {
            Ok(cfgs) => {
                for c in cfgs {
                    println!(
                        "   {:?} ch={} {}-{}Hz buffer={:?}",
                        c.sample_format(),
                        c.channels(),
                        c.min_sample_rate().0,
                        c.max_sample_rate().0,
                        c.buffer_size()
                    );
                }
            }
            Err(e) => println!("   configs err: {e}"),
        }
        match d.default_output_config() {
            Ok(c) => println!(
                "   DEFAULT: {:?} ch={} {}Hz",
                c.sample_format(),
                c.channels(),
                c.sample_rate().0
            ),
            Err(e) => println!("   default err: {e}"),
        }
    }
}
