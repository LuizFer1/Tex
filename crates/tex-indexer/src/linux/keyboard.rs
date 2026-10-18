use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::thread;
use std::time::Duration;

use crate::paste::{EV_KEY, KeyEvent, paste_steps};

const UINPUT: &str = "/dev/uinput";
const NAME: &[u8] = b"Tex virtual keyboard";
const BUS_VIRTUAL: u16 = 0x06;
const LAST_KEY: u16 = 248;
const STEP_GAP: Duration = Duration::from_millis(10);

const UI_DEV_CREATE: libc::Ioctl = 0x5501;
const UI_DEV_SETUP: libc::Ioctl = ioc_write(3, size_of::<libc::uinput_setup>());
const UI_SET_EVBIT: libc::Ioctl = ioc_write(100, size_of::<libc::c_int>());
const UI_SET_KEYBIT: libc::Ioctl = ioc_write(101, size_of::<libc::c_int>());

/// `_IOW('U', nr, size)` of the kernel's `uinput.h`.
const fn ioc_write(nr: u32, size: usize) -> libc::Ioctl {
    ((1 << 30) | ((size as u32) << 16) | (0x55 << 8) | nr) as libc::Ioctl
}

/// A keyboard made through `uinput`, which the desktop takes as one more keyboard. Needs root;
/// the device goes away when the file closes.
pub struct Keyboard {
    device: File,
}

impl Keyboard {
    /// Creates the device with keys 1 to 248, so udev tags it as a keyboard.
    pub fn create() -> io::Result<Keyboard> {
        let device = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(UINPUT)?;
        let fd = device.as_raw_fd();
        set_bit(fd, UI_SET_EVBIT, EV_KEY)?;
        for key in 1..=LAST_KEY {
            set_bit(fd, UI_SET_KEYBIT, key)?;
        }
        let setup = setup();
        check(unsafe { libc::ioctl(fd, UI_DEV_SETUP, &raw const setup) })?;
        check(unsafe { libc::ioctl(fd, UI_DEV_CREATE) })?;
        Ok(Keyboard { device })
    }

    /// Presses Ctrl+V, writing each step on its own a moment apart, so the desktop sees the
    /// keys in order.
    pub fn paste(&self) -> io::Result<()> {
        for (position, step) in paste_steps().iter().enumerate() {
            if position > 0 {
                thread::sleep(STEP_GAP);
            }
            (&self.device).write_all(&encode(step))?;
        }
        Ok(())
    }
}

fn setup() -> libc::uinput_setup {
    let mut setup = libc::uinput_setup {
        id: libc::input_id {
            bustype: BUS_VIRTUAL,
            vendor: 0x1234,
            product: 0x5678,
            version: 1,
        },
        name: [0; libc::UINPUT_MAX_NAME_SIZE],
        ff_effects_max: 0,
    };
    for (slot, byte) in setup.name.iter_mut().zip(NAME) {
        *slot = *byte as libc::c_char;
    }
    setup
}

fn set_bit(fd: RawFd, request: libc::Ioctl, bit: u16) -> io::Result<()> {
    check(unsafe { libc::ioctl(fd, request, libc::c_int::from(bit)) })
}

fn check(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// `events` as the kernel's `struct input_event`s; uinput ignores their time.
fn encode(events: &[KeyEvent]) -> Vec<u8> {
    let size = size_of::<libc::input_event>();
    let mut bytes = Vec::with_capacity(events.len() * size);
    for event in events {
        let input = libc::input_event {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: event.kind,
            code: event.code,
            value: event.value,
        };
        bytes.extend_from_slice(unsafe {
            std::slice::from_raw_parts((&raw const input).cast::<u8>(), size)
        });
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paste::{EV_SYN, KEY_V, SYN_REPORT};

    #[test]
    fn ioctl_numbers_match_the_kernel() {
        assert_eq!(UI_DEV_SETUP, 0x405c_5503);
        assert_eq!(UI_SET_EVBIT, 0x4004_5564);
        assert_eq!(UI_SET_KEYBIT, 0x4004_5565);
    }

    #[test]
    fn events_are_written_as_input_events() {
        let bytes = encode(&[
            KeyEvent {
                kind: EV_KEY,
                code: KEY_V,
                value: 1,
            },
            KeyEvent {
                kind: EV_SYN,
                code: SYN_REPORT,
                value: 0,
            },
        ]);
        assert_eq!(bytes.len(), 48);
        assert_eq!(&bytes[..16], &[0; 16]);
        assert_eq!(&bytes[16..24], &[1, 0, 47, 0, 1, 0, 0, 0]);
        assert_eq!(&bytes[24..48], &[0; 24]);
    }

    #[test]
    fn the_device_is_a_named_virtual_keyboard() {
        let setup = setup();
        assert_eq!(setup.id.bustype, BUS_VIRTUAL);
        let name: Vec<u8> = setup
            .name
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        assert_eq!(name, NAME);
    }
}
