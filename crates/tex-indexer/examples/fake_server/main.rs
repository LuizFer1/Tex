//! Development tool: indexes one folder with a plain directory walk and serves it on the real
//! Tex pipe, so the app can be tried without administrator rights or the service.
//! Usage: cargo run --release -p tex-indexer --example fake_server -- C:\Users\you

#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn main() {
    windows::main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("fake_server: not available on this system yet");
}
