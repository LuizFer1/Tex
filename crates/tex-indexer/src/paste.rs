/// logind's state file of the first seat, with `ACTIVE_UID=<uid>` while a session is active.
pub const SEAT0: &str = "/run/systemd/seats/seat0";

pub const EV_SYN: u16 = 0;
pub const EV_KEY: u16 = 1;
pub const SYN_REPORT: u16 = 0;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_V: u16 = 47;

/// One input event, as `struct input_event` carries it without the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

/// The user of the active session in logind's seat file, from its `ACTIVE_UID=` line.
pub fn active_uid(seat: &str) -> Option<u32> {
    seat.lines()
        .find_map(|line| line.strip_prefix("ACTIVE_UID="))?
        .trim()
        .parse()
        .ok()
}

/// Only the user in front of the screen may paste, so that another user (root included) can
/// not type into that session.
pub fn may_paste(uid: u32, active: Option<u32>) -> bool {
    active == Some(uid)
}

/// Ctrl down, V down, V up, Ctrl up: each step a key event and the report that delivers it.
pub fn paste_steps() -> [[KeyEvent; 2]; 4] {
    [
        key(KEY_LEFTCTRL, 1),
        key(KEY_V, 1),
        key(KEY_V, 0),
        key(KEY_LEFTCTRL, 0),
    ]
}

fn key(code: u16, value: i32) -> [KeyEvent; 2] {
    [
        KeyEvent {
            kind: EV_KEY,
            code,
            value,
        },
        KeyEvent {
            kind: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEAT: &str = "# This is private data. Do not parse.
IS_SEAT0=1
CAN_MULTI_SESSION=1
CAN_TTY=1
CAN_GRAPHICAL=1
ACTIVE=2
ACTIVE_UID=1000
SESSIONS=2 c1
UIDS=1000 120
";

    #[test]
    fn reads_the_active_user_of_the_seat() {
        assert_eq!(active_uid(SEAT), Some(1000));
        assert_eq!(active_uid("ACTIVE_UID=0\n"), Some(0));
        assert_eq!(active_uid("ACTIVE_UID=1001\r\n"), Some(1001));
    }

    #[test]
    fn a_seat_without_an_active_user_has_none() {
        assert_eq!(active_uid("IS_SEAT0=1\nUIDS=1000 120\n"), None);
        assert_eq!(active_uid("ACTIVE_UID=\n"), None);
        assert_eq!(active_uid("ACTIVE_UID=ana\n"), None);
        assert_eq!(active_uid("XACTIVE_UID=1000\n"), None);
        assert_eq!(active_uid(""), None);
    }

    #[test]
    fn only_the_active_user_may_paste() {
        assert!(may_paste(1000, Some(1000)));
        assert!(!may_paste(1001, Some(1000)));
        assert!(!may_paste(0, Some(1000)));
        assert!(may_paste(0, Some(0)));
        assert!(!may_paste(1000, None));
    }

    #[test]
    fn paste_presses_and_releases_ctrl_around_v() {
        let keys: Vec<(u16, u16, i32)> = paste_steps()
            .iter()
            .map(|step| (step[0].kind, step[0].code, step[0].value))
            .collect();
        assert_eq!(
            keys,
            [
                (EV_KEY, KEY_LEFTCTRL, 1),
                (EV_KEY, KEY_V, 1),
                (EV_KEY, KEY_V, 0),
                (EV_KEY, KEY_LEFTCTRL, 0),
            ]
        );
    }

    #[test]
    fn each_step_ends_with_a_report() {
        for step in paste_steps() {
            assert_eq!(
                step[1],
                KeyEvent {
                    kind: EV_SYN,
                    code: SYN_REPORT,
                    value: 0,
                }
            );
        }
    }
}
