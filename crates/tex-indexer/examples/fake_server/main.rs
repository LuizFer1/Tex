//! Development tool: indexes one folder with a plain directory walk and serves it like the
//! service, so the app can be tried without administrator rights or root.
//! Windows: cargo run --release -p tex-indexer --example fake_server -- C:\Users\you
//! Linux (debug build, socket of TEX_INDEXER_SOCKET):
//! TEX_INDEXER_SOCKET=/tmp/tex-dev.sock cargo run -p tex-indexer --example fake_server -- $HOME

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn main() {
    windows::main();
}

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(not(any(windows, target_os = "linux")))]
fn main() {
    eprintln!("fake_server: not available on this system yet");
}
