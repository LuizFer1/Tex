//! Development aid: runs Tex's clipboard watcher without the app. `watch [seconds]` prints every
//! copy (`watch-own` also reads Tex's own copies, to test two processes); `text <text> [seconds]`
//! and `png <file> [seconds]` copy as Tex does and keep serving the copy until the time is up.
//! On Linux `TEX_CLIPBOARD=x11|wayland|shell` picks the backend.

#[cfg(target_os = "linux")]
fn main() {
    use std::thread;
    use std::time::Duration;

    use tex_platform::{ClipboardWatcher, Payload, write_image, write_text};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds = |index: usize| {
        args.get(index)
            .and_then(|value| value.parse().ok())
            .unwrap_or(30)
    };
    let own = args.first().is_some_and(|command| command == "watch-own");
    let watcher = ClipboardWatcher::start(
        move |formats| {
            println!(
                "formats: text {} image {} excluded {} from_tex {} owner {:?}",
                formats.text, formats.image, formats.excluded, formats.from_tex, formats.owner_exe
            );
            if (formats.from_tex && !own) || formats.excluded {
                None
            } else if formats.text {
                Some(Payload::Text)
            } else if formats.image {
                Some(Payload::Image)
            } else {
                None
            }
        },
        |clip| match (&clip.text, &clip.png) {
            (Some(text), _) => println!("text: {text}"),
            (None, Some(png)) => println!("png: {} bytes", png.len()),
            _ => println!("empty"),
        },
    );
    let watcher = match watcher {
        Ok(watcher) => watcher,
        Err(error) => {
            eprintln!("clipboard: {error}");
            return;
        }
    };
    println!(
        "watching (knows owner: {}, through the GNOME extension: {})",
        watcher.knows_owner(),
        watcher.through_shell()
    );
    let wait = match args.first().map(String::as_str) {
        Some("text") => {
            let text = args.get(1).cloned().unwrap_or_default();
            println!("write: {:?}", write_text(&text));
            seconds(2)
        }
        Some("png") => {
            let png = std::fs::read(args.get(1).cloned().unwrap_or_default()).unwrap_or_default();
            println!("write: {:?}", write_image(&[], &png));
            seconds(2)
        }
        _ => seconds(1),
    };
    thread::sleep(Duration::from_secs(wait));
    drop(watcher);
    println!("stopped");
}

#[cfg(not(target_os = "linux"))]
fn main() {}
