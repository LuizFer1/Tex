use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use slint::platform::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: &'static KeyInfo,
}

#[derive(Debug, PartialEq, Eq)]
pub struct KeyInfo {
    pub name: &'static str,
    pub label: &'static str,
    pub code: Code,
}

macro_rules! keys {
    ($($name:literal $label:literal $code:ident),* $(,)?) => {
        &[$(KeyInfo { name: $name, label: $label, code: Code::$code }),*]
    };
}

const KEYS: &[KeyInfo] = keys![
    "a" "A" KeyA, "b" "B" KeyB, "c" "C" KeyC, "d" "D" KeyD, "e" "E" KeyE, "f" "F" KeyF,
    "g" "G" KeyG, "h" "H" KeyH, "i" "I" KeyI, "j" "J" KeyJ, "k" "K" KeyK, "l" "L" KeyL,
    "m" "M" KeyM, "n" "N" KeyN, "o" "O" KeyO, "p" "P" KeyP, "q" "Q" KeyQ, "r" "R" KeyR,
    "s" "S" KeyS, "t" "T" KeyT, "u" "U" KeyU, "v" "V" KeyV, "w" "W" KeyW, "x" "X" KeyX,
    "y" "Y" KeyY, "z" "Z" KeyZ,
    "0" "0" Digit0, "1" "1" Digit1, "2" "2" Digit2, "3" "3" Digit3, "4" "4" Digit4,
    "5" "5" Digit5, "6" "6" Digit6, "7" "7" Digit7, "8" "8" Digit8, "9" "9" Digit9,
    "f1" "F1" F1, "f2" "F2" F2, "f3" "F3" F3, "f4" "F4" F4, "f5" "F5" F5, "f6" "F6" F6,
    "f7" "F7" F7, "f8" "F8" F8, "f9" "F9" F9, "f10" "F10" F10, "f11" "F11" F11,
    "f12" "F12" F12, "f13" "F13" F13, "f14" "F14" F14, "f15" "F15" F15, "f16" "F16" F16,
    "f17" "F17" F17, "f18" "F18" F18, "f19" "F19" F19, "f20" "F20" F20, "f21" "F21" F21,
    "f22" "F22" F22, "f23" "F23" F23, "f24" "F24" F24,
    "space" "Space" Space, "up" "Up" ArrowUp, "down" "Down" ArrowDown, "left" "Left" ArrowLeft,
    "right" "Right" ArrowRight, "home" "Home" Home, "end" "End" End, "pageup" "PageUp" PageUp,
    "pagedown" "PageDown" PageDown, "insert" "Insert" Insert, "delete" "Delete" Delete,
    "comma" "," Comma, "period" "." Period, "slash" "/" Slash, "semicolon" ";" Semicolon,
    "quote" "'" Quote, "bracketleft" "[" BracketLeft, "bracketright" "]" BracketRight,
    "backslash" "\\" Backslash, "minus" "-" Minus, "equal" "=" Equal, "backquote" "`" Backquote,
];

fn key_named(name: &str) -> Option<&'static KeyInfo> {
    KEYS.iter().find(|key| key.name == name)
}

impl Shortcut {
    /// Reads "ctrl+shift+k" in any case and order. Needs at least one modifier and exactly one key.
    pub fn parse(text: &str) -> Option<Shortcut> {
        let mut shortcut = Shortcut {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: &KEYS[0],
        };
        let mut key = None;
        for part in text.split('+') {
            match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" => shortcut.ctrl = true,
                "alt" => shortcut.alt = true,
                "shift" => shortcut.shift = true,
                "win" | "super" | "meta" => shortcut.win = true,
                name => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(key_named(name)?);
                }
            }
        }
        shortcut.key = key?;
        shortcut.has_modifier().then_some(shortcut)
    }

    /// Builds a shortcut from a Slint key event; `None` for a lone modifier or an unsupported key.
    pub fn from_slint(
        text: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
        win: bool,
    ) -> Option<Shortcut> {
        let mut chars = text.chars();
        let first = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let key = match special_key(first) {
            Some(name) => key_named(name)?,
            None => KEYS
                .iter()
                .find(|key| key.label.chars().eq(first.to_uppercase()))?,
        };
        let shortcut = Shortcut {
            ctrl,
            alt,
            shift,
            win,
            key,
        };
        shortcut.has_modifier().then_some(shortcut)
    }

    fn has_modifier(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.win
    }

    fn modifier_names(&self) -> impl Iterator<Item = &'static str> {
        [
            (self.ctrl, "ctrl"),
            (self.alt, "alt"),
            (self.shift, "shift"),
            (self.win, "win"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
    }

    pub fn canonical(&self) -> String {
        self.modifier_names()
            .chain([self.key.name])
            .collect::<Vec<_>>()
            .join("+")
    }

    pub fn display(&self) -> String {
        self.modifier_names()
            .map(|name| match name {
                "ctrl" => "Ctrl",
                "alt" => "Alt",
                "shift" => "Shift",
                _ => "Win",
            })
            .chain([self.key.label])
            .collect::<Vec<_>>()
            .join("+")
    }

    pub fn hotkey(&self) -> HotKey {
        let mut modifiers = Modifiers::empty();
        modifiers.set(Modifiers::CONTROL, self.ctrl);
        modifiers.set(Modifiers::ALT, self.alt);
        modifiers.set(Modifiers::SHIFT, self.shift);
        modifiers.set(Modifiers::SUPER, self.win);
        HotKey::new(Some(modifiers), self.key.code)
    }
}

fn special_key(c: char) -> Option<&'static str> {
    const SPECIAL: &[(Key, &str)] = &[
        (Key::F1, "f1"),
        (Key::F2, "f2"),
        (Key::F3, "f3"),
        (Key::F4, "f4"),
        (Key::F5, "f5"),
        (Key::F6, "f6"),
        (Key::F7, "f7"),
        (Key::F8, "f8"),
        (Key::F9, "f9"),
        (Key::F10, "f10"),
        (Key::F11, "f11"),
        (Key::F12, "f12"),
        (Key::F13, "f13"),
        (Key::F14, "f14"),
        (Key::F15, "f15"),
        (Key::F16, "f16"),
        (Key::F17, "f17"),
        (Key::F18, "f18"),
        (Key::F19, "f19"),
        (Key::F20, "f20"),
        (Key::F21, "f21"),
        (Key::F22, "f22"),
        (Key::F23, "f23"),
        (Key::F24, "f24"),
        (Key::Space, "space"),
        (Key::UpArrow, "up"),
        (Key::DownArrow, "down"),
        (Key::LeftArrow, "left"),
        (Key::RightArrow, "right"),
        (Key::Home, "home"),
        (Key::End, "end"),
        (Key::PageUp, "pageup"),
        (Key::PageDown, "pagedown"),
        (Key::Insert, "insert"),
        (Key::Delete, "delete"),
    ];
    SPECIAL
        .iter()
        .find(|(key, _)| char::from(key.clone()) == c)
        .map(|(_, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Shortcut {
        Shortcut::parse(text).unwrap()
    }

    #[test]
    fn parses_and_formats() {
        let shortcut = parse("alt+space");
        assert_eq!(shortcut.canonical(), "alt+space");
        assert_eq!(shortcut.display(), "Alt+Space");
        assert_eq!(parse("ctrl+comma").display(), "Ctrl+,");
    }

    #[test]
    fn order_and_case_do_not_matter() {
        assert_eq!(parse("Shift+CTRL+k").canonical(), "ctrl+shift+k");
        assert_eq!(parse("win+Alt+F5").display(), "Alt+Win+F5");
    }

    #[test]
    fn rejects_without_modifier_or_key() {
        for text in ["k", "ctrl+alt", "ctrl+nope", "", "ctrl+a+b"] {
            assert_eq!(Shortcut::parse(text), None, "{text}");
        }
    }

    #[test]
    fn maps_to_global_hotkey() {
        assert_eq!(
            parse("alt+space").hotkey(),
            HotKey::new(Some(Modifiers::ALT), Code::Space)
        );
        assert_eq!(
            parse("win+f1").hotkey(),
            HotKey::new(Some(Modifiers::SUPER), Code::F1)
        );
    }

    #[test]
    fn maps_slint_events() {
        let from = |text: &str, ctrl, alt| {
            Shortcut::from_slint(text, ctrl, alt, false, false).map(|s| s.canonical())
        };
        assert_eq!(from("K", true, false).as_deref(), Some("ctrl+k"));
        assert_eq!(from("7", true, false).as_deref(), Some("ctrl+7"));
        let f5 = char::from(Key::F5).to_string();
        assert_eq!(from(&f5, false, true).as_deref(), Some("alt+f5"));
        assert_eq!(from(",", true, false).as_deref(), Some("ctrl+comma"));
        assert_eq!(from(" ", false, true).as_deref(), Some("alt+space"));
        assert_eq!(from("k", false, false), None);
        let control = char::from(Key::Control).to_string();
        assert_eq!(from(&control, true, false), None);
    }
}
