//! One-shot proof of the update check against the live GitHub API.
#[path = "../src/update.rs"]
mod update;

fn main() {
    match update::check_latest() {
        Ok(None) => println!("no newer release"),
        Ok(Some(r)) => println!("NEWER: {} {} {}", r.tag, r.name, r.zip_url),
        Err(e) => println!("check note: {e}"),
    }
}
