//! Prints the size of the icon Tex shows for an app: a `.desktop` file on Linux, a program or
//! shortcut on Windows. Usage: cargo run -p tex-platform --example app_icon -- <path>

fn main() {
    let path = std::env::args().nth(1).expect("usage: app_icon <path>");
    tex_platform::init_icon_thread();
    match tex_platform::app_icon(&path) {
        Some(icon) => println!("{}x{} icon", icon.width, icon.height),
        None => {
            println!("no icon");
            std::process::exit(1);
        }
    }
}
