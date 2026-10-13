fn main() {
    println!("-- inputs --");
    for (id, name) in audio_engine::input_devices() {
        println!("  {name}  [id={id}]");
    }
    println!("-- outputs --");
    for (id, name) in audio_engine::output_devices() {
        println!("  {name}  [id={id}]");
    }
}
