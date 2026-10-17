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

#[derive(Debug, Clone, Default)]
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
        && settings
            .ignored_apps
            .iter()
            .any(|app| app.eq_ignore_ascii_case(owner))
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

pub fn fits(size: u64, settings: &CaptureSettings) -> bool {
    size <= settings.max_item_bytes
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
}
