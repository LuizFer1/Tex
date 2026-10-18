use std::path::Path;

use crate::{Clip, ClipFormats, Payload};

/// Marks Tex's own copies, so the watcher does not capture them again.
pub(crate) const TEX_MIME: &str = "application/x-tex-copy";
/// Set by password managers (KeePassXC and others); the value `secret` opts the copy out.
pub(crate) const HINT_MIME: &str = "x-kde-passwordManagerHint";
pub(crate) const PNG_MIME: &str = "image/png";
/// Text targets, best first. All of them carry UTF-8 in practice.
pub(crate) const TEXT_MIMES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

/// How Tex reaches the clipboard in this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backend {
    X11,
    DataControl,
    Shell,
}

/// The backends to try, in order. `forced` (`TEX_CLIPBOARD` in debug builds: `x11`, `wayland`
/// or `shell`) picks one; otherwise X11 when the app's windows are X11, else the Wayland
/// data-control protocols and, on GNOME, the extension.
pub(crate) fn backends(
    forced: Option<&str>,
    x11: bool,
    wayland: bool,
    gnome: bool,
) -> Vec<Backend> {
    match forced {
        Some("x11") => return vec![Backend::X11],
        Some("wayland") => return vec![Backend::DataControl],
        Some("shell") => return vec![Backend::Shell],
        _ => {}
    }
    if x11 {
        return vec![Backend::X11];
    }
    let mut list = Vec::new();
    if wayland {
        list.push(Backend::DataControl);
    }
    if gnome {
        list.push(Backend::Shell);
    }
    list
}

/// The best text target among `mimes`.
pub(crate) fn text_mime(mimes: &[String]) -> Option<&'static str> {
    TEXT_MIMES
        .into_iter()
        .find(|wanted| mimes.iter().any(|mime| mime == wanted))
}

fn offers(mimes: &[String], wanted: &str) -> bool {
    mimes.iter().any(|mime| mime == wanted)
}

/// What the formats alone say, before the password hint is read.
pub(crate) fn formats(mimes: &[String], owner_exe: Option<String>) -> ClipFormats {
    ClipFormats {
        text: text_mime(mimes).is_some(),
        image: offers(mimes, PNG_MIME),
        excluded: false,
        history_disallowed: false,
        from_tex: offers(mimes, TEX_MIME),
        owner_exe,
    }
}

/// Whether the password hint opts the copy out. A hint that could not be read counts as `secret`.
pub(crate) fn says_secret(value: Option<&[u8]>) -> bool {
    value.is_none_or(|value| value.trim_ascii() == b"secret")
}

/// Text as the clipboard carries it: UTF-8 (anything else is replaced) without trailing NULs.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .to_owned()
}

/// Reads one clipboard change: the formats first, then only what `wanted` asks for, through
/// `fetch` (which returns the bytes of one target). The password hint is read only when the
/// copy would otherwise be captured. `None` when nothing is wanted or the content is empty.
pub(crate) fn read(
    mimes: &[String],
    owner_exe: Option<String>,
    wanted: &dyn Fn(&ClipFormats) -> Option<Payload>,
    fetch: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> Option<Clip> {
    let mut formats = formats(mimes, owner_exe);
    let mut payload = wanted(&formats)?;
    if offers(mimes, HINT_MIME) && says_secret(fetch(HINT_MIME).as_deref()) {
        formats.excluded = true;
        payload = wanted(&formats)?;
    }
    let mut clip = Clip {
        excluded: formats.excluded,
        from_tex: formats.from_tex,
        owner_exe: formats.owner_exe,
        ..Clip::default()
    };
    match payload {
        Payload::Text => {
            let text = decode_text(&fetch(text_mime(mimes)?)?);
            if text.is_empty() {
                return None;
            }
            clip.text = Some(text);
        }
        Payload::Image => {
            let png = fetch(PNG_MIME).filter(|png| !png.is_empty())?;
            clip.png = Some(png);
        }
    }
    Some(clip)
}

/// What Tex offers when it copies text: the text targets and its mark.
pub(crate) fn text_offers(text: &str) -> Vec<(String, Vec<u8>)> {
    let mut items: Vec<(String, Vec<u8>)> = TEXT_MIMES
        .iter()
        .map(|mime| (mime.to_string(), text.as_bytes().to_vec()))
        .collect();
    items.push((TEX_MIME.to_owned(), vec![1]));
    items
}

/// What Tex offers when it copies an image: the PNG and its mark.
pub(crate) fn png_offers(png: &[u8]) -> Vec<(String, Vec<u8>)> {
    vec![
        (PNG_MIME.to_owned(), png.to_vec()),
        (TEX_MIME.to_owned(), vec![1]),
    ]
}

/// The executable's file name from the target of `/proc/<pid>/exe`.
pub(crate) fn exe_name(link: &Path) -> Option<String> {
    let name = link.file_name()?.to_string_lossy();
    let name = name.strip_suffix(" (deleted)").unwrap_or(&name);
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::*;

    fn mimes(list: &[&str]) -> Vec<String> {
        list.iter().map(|mime| mime.to_string()).collect()
    }

    fn take_all(formats: &ClipFormats) -> Option<Payload> {
        if formats.from_tex || formats.excluded {
            None
        } else if formats.text {
            Some(Payload::Text)
        } else if formats.image {
            Some(Payload::Image)
        } else {
            None
        }
    }

    struct Source {
        data: Vec<(&'static str, &'static [u8])>,
        asked: RefCell<Vec<String>>,
    }

    impl Source {
        fn new(data: Vec<(&'static str, &'static [u8])>) -> Source {
            Source {
                data,
                asked: RefCell::new(Vec::new()),
            }
        }

        fn mimes(&self) -> Vec<String> {
            self.data.iter().map(|(mime, _)| mime.to_string()).collect()
        }

        fn read(&self, wanted: &dyn Fn(&ClipFormats) -> Option<Payload>) -> Option<Clip> {
            let mut fetch = |mime: &str| {
                self.asked.borrow_mut().push(mime.to_owned());
                self.data
                    .iter()
                    .find(|(offered, _)| *offered == mime)
                    .map(|(_, bytes)| bytes.to_vec())
            };
            read(&self.mimes(), Some("gedit".into()), wanted, &mut fetch)
        }

        fn asked(&self) -> Vec<String> {
            self.asked.borrow().clone()
        }
    }

    #[test]
    fn x11_wins_then_data_control_then_the_extension() {
        assert_eq!(backends(None, true, true, true), [Backend::X11]);
        assert_eq!(
            backends(None, false, true, true),
            [Backend::DataControl, Backend::Shell]
        );
        assert_eq!(backends(None, false, true, false), [Backend::DataControl]);
        assert_eq!(backends(None, false, false, false), []);
    }

    #[test]
    fn a_forced_backend_is_the_only_one() {
        assert_eq!(
            backends(Some("shell"), true, false, false),
            [Backend::Shell]
        );
        assert_eq!(
            backends(Some("wayland"), true, false, false),
            [Backend::DataControl]
        );
        assert_eq!(backends(Some("x11"), false, true, true), [Backend::X11]);
        assert_eq!(backends(Some("other"), true, false, false), [Backend::X11]);
    }

    #[test]
    fn utf8_text_targets_come_first() {
        assert_eq!(
            text_mime(&mimes(&["TEXT", "text/plain", "UTF8_STRING"])),
            Some("UTF8_STRING")
        );
        assert_eq!(
            text_mime(&mimes(&["text/plain", "text/plain;charset=utf-8"])),
            Some("text/plain;charset=utf-8")
        );
        assert_eq!(text_mime(&mimes(&["STRING", "TEXT", "image/png"])), None);
    }

    #[test]
    fn formats_come_from_the_mime_list() {
        let formats = formats(&mimes(&["image/png", TEX_MIME]), Some("gimp".into()));
        assert!(!formats.text);
        assert!(formats.image);
        assert!(formats.from_tex);
        assert!(!formats.excluded && !formats.history_disallowed);
        assert_eq!(formats.owner_exe.as_deref(), Some("gimp"));
    }

    #[test]
    fn only_the_secret_hint_opts_out() {
        assert!(says_secret(Some(b"secret")));
        assert!(says_secret(Some(b" secret\n")));
        assert!(says_secret(None));
        assert!(!says_secret(Some(b"other")));
        assert!(!says_secret(Some(b"")));
    }

    #[test]
    fn text_is_utf8_without_trailing_nuls() {
        assert_eq!(decode_text(b"ol\xc3\xa1\0\0"), "olá");
        assert_eq!(decode_text(b"a\xffb"), "a\u{fffd}b");
    }

    #[test]
    fn reads_only_the_wanted_text() {
        let source = Source::new(vec![
            ("text/plain", b"plain"),
            ("UTF8_STRING", b"hello"),
            ("image/png", b"png"),
        ]);
        let clip = source.read(&take_all).unwrap();
        assert_eq!(clip.text.as_deref(), Some("hello"));
        assert_eq!(clip.png, None);
        assert_eq!(clip.dib, None);
        assert_eq!(clip.owner_exe.as_deref(), Some("gedit"));
        assert_eq!(source.asked(), ["UTF8_STRING"]);
    }

    #[test]
    fn reads_the_png_when_the_image_is_wanted() {
        let source = Source::new(vec![("image/png", b"\x89PNG")]);
        let clip = source.read(&take_all).unwrap();
        assert_eq!(clip.png.as_deref(), Some(&b"\x89PNG"[..]));
        assert_eq!(clip.text, None);
        assert_eq!(source.asked(), ["image/png"]);
    }

    #[test]
    fn nothing_is_read_when_nothing_is_wanted() {
        let source = Source::new(vec![("UTF8_STRING", b"hi"), (HINT_MIME, b"secret")]);
        assert_eq!(source.read(&|_| None), None);
        assert!(source.asked().is_empty());
        let own = Source::new(vec![("UTF8_STRING", b"hi"), (TEX_MIME, b"\x01")]);
        assert_eq!(own.read(&take_all), None);
        assert!(own.asked().is_empty());
    }

    #[test]
    fn a_secret_hint_marks_the_copy_excluded() {
        let source = Source::new(vec![(HINT_MIME, b"secret"), ("UTF8_STRING", b"pw")]);
        assert_eq!(source.read(&take_all), None);
        assert_eq!(source.asked(), [HINT_MIME]);
        let kept = source.read(&|formats| formats.text.then_some(Payload::Text));
        assert!(kept.unwrap().excluded);
    }

    #[test]
    fn another_hint_value_is_captured() {
        let source = Source::new(vec![(HINT_MIME, b"public"), ("UTF8_STRING", b"hi")]);
        let clip = source.read(&take_all).unwrap();
        assert!(!clip.excluded);
        assert_eq!(clip.text.as_deref(), Some("hi"));
    }

    #[test]
    fn empty_content_is_not_a_copy() {
        let empty = Source::new(vec![("UTF8_STRING", b"\0")]);
        assert_eq!(empty.read(&take_all), None);
        let empty_png = Source::new(vec![("image/png", b"")]);
        assert_eq!(empty_png.read(&take_all), None);
    }

    #[test]
    fn tex_offers_its_mark_with_the_content() {
        let text = text_offers("hi");
        assert_eq!(text.len(), TEXT_MIMES.len() + 1);
        assert!(text.contains(&("UTF8_STRING".to_owned(), b"hi".to_vec())));
        assert!(text.contains(&(TEX_MIME.to_owned(), vec![1])));
        assert_eq!(
            png_offers(b"png"),
            [
                (PNG_MIME.to_owned(), b"png".to_vec()),
                (TEX_MIME.to_owned(), vec![1])
            ]
        );
    }

    #[test]
    fn the_exe_name_comes_from_the_link() {
        assert_eq!(
            exe_name(&PathBuf::from("/usr/bin/keepassxc")).as_deref(),
            Some("keepassxc")
        );
        assert_eq!(
            exe_name(&PathBuf::from("/opt/app/app (deleted)")).as_deref(),
            Some("app")
        );
        assert_eq!(exe_name(&PathBuf::from("/")), None);
    }
}
