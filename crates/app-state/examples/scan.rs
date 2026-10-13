fn main() {
    let folder = std::env::args()
        .nth(1)
        .unwrap_or_else(|| r"C:\Users\Administrator\AppData\Local\hermes\cache\scratch\VST-Applier\common\VST".into());
    let found = app_state::scan(std::path::Path::new(&folder));
    println!("found {} plugins in {folder}", found.len());
    for c in &found {
        println!(
            "  [{:?}] {} -> {}",
            c.format,
            c.name,
            c.path.display()
        );
    }
}
