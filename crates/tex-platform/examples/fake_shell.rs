//! Development aid: stands in for Tex's GNOME Shell extension on the session bus, so the app's
//! GNOME path runs without GNOME (start the app with `XDG_CURRENT_DESKTOP=GNOME`). Prints every
//! call it gets. With `--press` it reports the Tex shortcut once, two seconds after the app
//! first hands it the shortcuts. With `--clip` it announces a text copy by `gedit` every three
//! seconds (`fake copy <n>`), as the extension does with `Offer`; start the app or the
//! `clipboard` example with `TEX_CLIPBOARD=shell` to read them.

#[cfg(target_os = "linux")]
mod fake {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::thread;
    use std::time::Duration;

    use zbus::blocking::connection::Builder;
    use zbus::{Connection, interface};

    const NAME: &str = "io.github.LuizFer1.Tex.Shell";
    const PATH: &str = "/io/github/LuizFer1/Tex/Shell";
    const INTERFACE: &str = "io.github.LuizFer1.Tex.Shell";

    struct Shell {
        press: AtomicBool,
        serial: AtomicU32,
    }

    #[interface(name = "io.github.LuizFer1.Tex.Shell")]
    impl Shell {
        fn work_area_under_cursor(&self) -> (i32, i32, u32, u32, f64) {
            println!("WorkAreaUnderCursor");
            (0, 0, 1920, 1040, 1.0)
        }

        fn primary_work_area(&self) -> (i32, i32, u32, u32, f64) {
            println!("PrimaryWorkArea");
            (0, 0, 1920, 1040, 1.0)
        }

        fn present(&self, title: &str, x: i32, y: i32) {
            println!("Present {title} {x} {y}");
        }

        fn set_shortcuts(
            &self,
            tex: &str,
            history: &str,
            #[zbus(connection)] connection: &Connection,
        ) -> (bool, bool) {
            println!("SetShortcuts {tex} {history}");
            if !tex.is_empty() && self.press.swap(false, Ordering::SeqCst) {
                let connection = connection.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(2));
                    println!("pressing the Tex shortcut");
                    let signal = connection.emit_signal(
                        None::<()>,
                        PATH,
                        INTERFACE,
                        "ShortcutActivated",
                        &("tex",),
                    );
                    let _ = zbus::block_on(signal);
                });
            }
            (true, true)
        }

        fn deliver(&self, serial: u32, mime: &str) -> Vec<u8> {
            println!("Deliver {serial} {mime}");
            if serial == self.serial.load(Ordering::SeqCst) {
                format!("fake copy {serial}").into_bytes()
            } else {
                Vec::new()
            }
        }

        fn set_clipboard(&self, mime: &str, data: Vec<u8>) {
            println!("SetClipboard {mime} {} bytes", data.len());
        }
    }

    pub fn main() -> zbus::Result<()> {
        let press = std::env::args().any(|arg| arg == "--press");
        let clip = std::env::args().any(|arg| arg == "--clip");
        let shell = Shell {
            press: AtomicBool::new(press),
            serial: AtomicU32::new(0),
        };
        let bus = Builder::session()?
            .name(NAME)?
            .serve_at(PATH, shell)?
            .build()?;
        println!("fake shell on the bus");
        loop {
            if !clip {
                thread::park();
                continue;
            }
            thread::sleep(Duration::from_secs(3));
            let shell = bus.object_server().interface::<_, Shell>(PATH)?;
            let serial = shell.get().serial.fetch_add(1, Ordering::SeqCst) + 1;
            println!("offering copy {serial}");
            let mimes = vec!["text/plain;charset=utf-8", "UTF8_STRING", "TEXT"];
            bus.emit_signal(
                None::<()>,
                PATH,
                INTERFACE,
                "Offer",
                &(serial, mimes, "gedit"),
            )?;
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> zbus::Result<()> {
    fake::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {}
