use std::hash::{DefaultHasher, Hash, Hasher};

use crate::image::Rgba;

#[derive(Debug, Clone, Default)]
pub struct ClipFacts<'a> {
    pub from_tex: bool,
    /// `ExcludeClipboardContentFromMonitorProcessing` is present.
    pub excluded: bool,
    /// `CanIncludeInClipboardHistory` is 0.
    pub history_disallowed: bool,
    pub owner_exe: Option<&'a str>,
    /// Non-empty `CF_UNICODETEXT`.
    pub has_text: bool,
    /// `CF_DIBV5` or `CF_DIB`.
    pub has_image: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureSettings {
    pub paused: bool,
    pub capture_text: bool,
    pub capture_images: bool,
    pub max_item_bytes: u64,
    pub respect_exclusion: bool,
    pub ignored_apps: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Text,
    Image,
    Skip(Skip),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    Paused,
    FromTex,
    OptedOut,
    IgnoredApp,
    NoContent,
}

/// Applies the capture rules in order: pause, Tex's own copies, opt-out formats,
/// ignored apps, then text before image.
pub fn should_capture(facts: &ClipFacts, settings: &CaptureSettings) -> Decision {
    if settings.paused {
        return Decision::Skip(Skip::Paused);
    }
    if facts.from_tex {
        return Decision::Skip(Skip::FromTex);
    }
    if settings.respect_exclusion && (facts.excluded || facts.history_disallowed) {
        return Decision::Skip(Skip::OptedOut);
    }
    if let Some(owner) = facts.owner_exe
        && settings.ignored_apps.iter().any(|app| same_app(app, owner))
    {
        return Decision::Skip(Skip::IgnoredApp);
    }
    if settings.capture_text && facts.has_text {
        Decision::Text
    } else if settings.capture_images && facts.has_image {
        Decision::Image
    } else {
        Decision::Skip(Skip::NoContent)
    }
}

/// Exe names compared the way Windows compares file names: without case, accents included.
pub fn same_app(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

pub fn fits(size: u64, settings: &CaptureSettings) -> bool {
    size <= settings.max_item_bytes
}

/// How long identical content still counts as the copy saved last.
pub const REPEAT_WINDOW_MS: i64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastCopy {
    pub fingerprint: u64,
    pub copied_at: i64,
}

pub fn text_fingerprint(text: &str) -> u64 {
    fingerprint(&(0u8, text))
}

pub fn image_fingerprint(image: &Rgba) -> u64 {
    fingerprint(&(1u8, image.width, image.height, &image.pixels))
}

fn fingerprint(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Apps often write one copy more than once (OLE writes and then flushes, some tools write
/// twice), so the same content within [`REPEAT_WINDOW_MS`] of the last saved copy is that copy.
pub fn is_repeat(last: Option<LastCopy>, fingerprint: u64, now: i64) -> bool {
    last.is_some_and(|last| {
        last.fingerprint == fingerprint && (0..=REPEAT_WINDOW_MS).contains(&(now - last.copied_at))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> CaptureSettings {
        CaptureSettings {
            paused: false,
            capture_text: true,
            capture_images: true,
            max_item_bytes: 10,
            respect_exclusion: true,
            ignored_apps: vec!["KeePass.exe".into()],
        }
    }

    fn text<'a>() -> ClipFacts<'a> {
        ClipFacts {
            owner_exe: Some("notepad.exe"),
            has_text: true,
            ..ClipFacts::default()
        }
    }

    #[test]
    fn captures_text() {
        assert_eq!(should_capture(&text(), &settings()), Decision::Text);
    }

    #[test]
    fn paused_wins() {
        let settings = CaptureSettings {
            paused: true,
            ..settings()
        };
        let facts = ClipFacts {
            from_tex: true,
            ..text()
        };
        assert_eq!(
            should_capture(&facts, &settings),
            Decision::Skip(Skip::Paused)
        );
    }

    #[test]
    fn skips_copies_from_tex() {
        let facts = ClipFacts {
            from_tex: true,
            excluded: true,
            ..text()
        };
        assert_eq!(
            should_capture(&facts, &settings()),
            Decision::Skip(Skip::FromTex)
        );
    }

    #[test]
    fn skips_apps_that_opt_out() {
        let excluded = ClipFacts {
            excluded: true,
            ..text()
        };
        let disallowed = ClipFacts {
            history_disallowed: true,
            owner_exe: Some("keepass.exe"),
            ..text()
        };
        assert_eq!(
            should_capture(&excluded, &settings()),
            Decision::Skip(Skip::OptedOut)
        );
        assert_eq!(
            should_capture(&disallowed, &settings()),
            Decision::Skip(Skip::OptedOut)
        );
    }

    #[test]
    fn opt_out_ignored_when_not_respected() {
        let settings = CaptureSettings {
            respect_exclusion: false,
            ..settings()
        };
        let facts = ClipFacts {
            excluded: true,
            history_disallowed: true,
            ..text()
        };
        assert_eq!(should_capture(&facts, &settings), Decision::Text);
    }

    #[test]
    fn skips_ignored_apps_ignoring_case() {
        let facts = ClipFacts {
            owner_exe: Some("KEEPASS.EXE"),
            ..text()
        };
        assert_eq!(
            should_capture(&facts, &settings()),
            Decision::Skip(Skip::IgnoredApp)
        );
        let facts = ClipFacts {
            owner_exe: None,
            ..text()
        };
        assert_eq!(should_capture(&facts, &settings()), Decision::Text);
        let mut settings = settings();
        settings.ignored_apps = vec!["Éditeur.exe".into()];
        let facts = ClipFacts {
            owner_exe: Some("éDITEUR.EXE"),
            ..text()
        };
        assert_eq!(
            should_capture(&facts, &settings),
            Decision::Skip(Skip::IgnoredApp)
        );
    }

    #[test]
    fn text_wins_over_image() {
        let facts = ClipFacts {
            has_image: true,
            ..text()
        };
        assert_eq!(should_capture(&facts, &settings()), Decision::Text);
    }

    #[test]
    fn falls_back_to_image_when_text_is_off() {
        let facts = ClipFacts {
            has_image: true,
            ..text()
        };
        let settings = CaptureSettings {
            capture_text: false,
            ..settings()
        };
        assert_eq!(should_capture(&facts, &settings), Decision::Image);
    }

    #[test]
    fn nothing_to_capture() {
        let empty = ClipFacts::default();
        assert_eq!(
            should_capture(&empty, &settings()),
            Decision::Skip(Skip::NoContent)
        );
        let settings = CaptureSettings {
            capture_images: false,
            ..settings()
        };
        let image = ClipFacts {
            has_image: true,
            ..ClipFacts::default()
        };
        assert_eq!(
            should_capture(&image, &settings),
            Decision::Skip(Skip::NoContent)
        );
    }

    #[test]
    fn fits_up_to_the_maximum() {
        assert!(fits(10, &settings()));
        assert!(!fits(11, &settings()));
    }

    fn last(text: &str, copied_at: i64) -> Option<LastCopy> {
        Some(LastCopy {
            fingerprint: text_fingerprint(text),
            copied_at,
        })
    }

    #[test]
    fn same_content_within_the_window_is_a_repeat() {
        let fingerprint = text_fingerprint("same");
        assert!(is_repeat(last("same", 5_000), fingerprint, 5_000));
        assert!(is_repeat(
            last("same", 5_000),
            fingerprint,
            5_000 + REPEAT_WINDOW_MS
        ));
    }

    #[test]
    fn same_content_after_the_window_is_a_new_copy() {
        let fingerprint = text_fingerprint("same");
        assert!(!is_repeat(
            last("same", 5_000),
            fingerprint,
            5_001 + REPEAT_WINDOW_MS
        ));
        assert!(!is_repeat(last("same", 5_000), fingerprint, 4_999));
    }

    #[test]
    fn other_content_or_no_last_copy_is_a_new_copy() {
        assert!(!is_repeat(
            last("same", 5_000),
            text_fingerprint("other"),
            5_000
        ));
        assert!(!is_repeat(None, text_fingerprint("same"), 5_000));
    }

    #[test]
    fn images_compare_size_and_pixels() {
        let image = Rgba {
            width: 2,
            height: 1,
            pixels: vec![1; 8],
        };
        let flipped = Rgba {
            width: 1,
            height: 2,
            ..image.clone()
        };
        assert_eq!(image_fingerprint(&image), image_fingerprint(&image.clone()));
        assert_ne!(image_fingerprint(&image), image_fingerprint(&flipped));
    }
}
