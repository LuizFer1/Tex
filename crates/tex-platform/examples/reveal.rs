//! Development tool: shows a path selected in the file manager, as Ctrl+Enter does in the popup.
//! Usage: cargo run -p tex-platform --example reveal -- /etc/hostname

use std::thread;
use std::time::Duration;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: reveal <path>");
        return;
    };
    if let Err(error) = tex_platform::reveal_in_folder(&path) {
        eprintln!("reveal: {error}");
    }
    thread::sleep(Duration::from_secs(5));
}
