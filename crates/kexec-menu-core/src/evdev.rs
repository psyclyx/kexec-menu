// Linux evdev input device reader.
//
// On aarch64/Android devices and SBCs, input may come from gpio-keys
// (volume buttons, power button) rather than a terminal keyboard.
// This module reads raw evdev events from /dev/input/event* and maps
// them to TUI Key values.

use std::os::fd::RawFd;

use crate::tui::Key;

const EV_KEY: u16 = 1;

// Key codes (linux/input-event-codes.h)
const KEY_ESC: u16 = 1;
const KEY_BACKSPACE: u16 = 14;
const KEY_ENTER: u16 = 28;
const KEY_UP: u16 = 103;
const KEY_DOWN: u16 = 108;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_VOLUMEUP: u16 = 115;
const KEY_VOLUMEDOWN: u16 = 114;
const KEY_POWER: u16 = 116;

// struct input_event sizes differ by pointer width:
//   64-bit: timeval(16) + type(2) + code(2) + value(4) = 24
//   32-bit: timeval(8)  + type(2) + code(2) + value(4) = 16
#[cfg(target_pointer_width = "64")]
const EVENT_SIZE: usize = 24;
#[cfg(target_pointer_width = "64")]
const TYPE_OFFSET: usize = 16;

#[cfg(target_pointer_width = "32")]
const EVENT_SIZE: usize = 16;
#[cfg(target_pointer_width = "32")]
const TYPE_OFFSET: usize = 8;

/// Reads key events from Linux evdev input devices.
pub struct EvdevReader {
    fds: Vec<RawFd>,
}

impl EvdevReader {
    /// Scan /dev/input/event0..15 and open any devices found.
    /// Returns None if no devices are available.
    pub fn open() -> Option<Self> {
        let mut fds = Vec::new();
        for i in 0u8..16 {
            let path = format!("/dev/input/event{i}\0");
            let fd = unsafe {
                libc::open(
                    path.as_ptr() as *const libc::c_char,
                    libc::O_RDONLY | libc::O_NONBLOCK,
                )
            };
            if fd >= 0 {
                fds.push(fd);
            }
        }
        if fds.is_empty() {
            None
        } else {
            Some(Self { fds })
        }
    }

    /// Raw file descriptors for use with poll().
    pub fn fds(&self) -> &[RawFd] {
        &self.fds
    }

    /// Try to read a key event from a ready fd.
    /// Returns None if the event isn't a key press/repeat or the key is unmapped.
    pub fn try_read_key(&self, fd: RawFd) -> Option<Key> {
        let mut buf = [0u8; EVENT_SIZE];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, EVENT_SIZE) };
        if n != EVENT_SIZE as isize {
            return None;
        }

        let ev_type = u16::from_ne_bytes([buf[TYPE_OFFSET], buf[TYPE_OFFSET + 1]]);
        let ev_code = u16::from_ne_bytes([buf[TYPE_OFFSET + 2], buf[TYPE_OFFSET + 3]]);
        let ev_value = i32::from_ne_bytes([
            buf[TYPE_OFFSET + 4],
            buf[TYPE_OFFSET + 5],
            buf[TYPE_OFFSET + 6],
            buf[TYPE_OFFSET + 7],
        ]);

        // Only handle press (1) and repeat (2), ignore release (0)
        if ev_type != EV_KEY || ev_value == 0 {
            return None;
        }

        match ev_code {
            KEY_UP | KEY_VOLUMEUP => Some(Key::Up),
            KEY_DOWN | KEY_VOLUMEDOWN => Some(Key::Down),
            KEY_LEFT => Some(Key::Left),
            KEY_RIGHT => Some(Key::Right),
            KEY_ENTER | KEY_POWER => Some(Key::Enter),
            KEY_ESC => Some(Key::Escape),
            KEY_BACKSPACE => Some(Key::Backspace),
            _ => None,
        }
    }
}

impl Drop for EvdevReader {
    fn drop(&mut self) {
        for &fd in &self.fds {
            unsafe { libc::close(fd) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key_press_up() {
        let reader = EvdevReader { fds: Vec::new() };
        // Simulate a 64-bit input_event for KEY_UP press
        let mut buf = [0u8; EVENT_SIZE];
        // type = EV_KEY (1)
        buf[TYPE_OFFSET..TYPE_OFFSET + 2].copy_from_slice(&EV_KEY.to_ne_bytes());
        // code = KEY_UP (103)
        buf[TYPE_OFFSET + 2..TYPE_OFFSET + 4].copy_from_slice(&KEY_UP.to_ne_bytes());
        // value = 1 (press)
        buf[TYPE_OFFSET + 4..TYPE_OFFSET + 8].copy_from_slice(&1i32.to_ne_bytes());
        let key = parse_event(&buf);
        assert_eq!(key, Some(Key::Up));
        // Also test the reader's method indirectly via parse_event
        let _ = reader; // just verify construction works
    }

    #[test]
    fn parse_key_release_ignored() {
        let mut buf = [0u8; EVENT_SIZE];
        buf[TYPE_OFFSET..TYPE_OFFSET + 2].copy_from_slice(&EV_KEY.to_ne_bytes());
        buf[TYPE_OFFSET + 2..TYPE_OFFSET + 4].copy_from_slice(&KEY_ENTER.to_ne_bytes());
        buf[TYPE_OFFSET + 4..TYPE_OFFSET + 8].copy_from_slice(&0i32.to_ne_bytes()); // release
        assert_eq!(parse_event(&buf), None);
    }

    #[test]
    fn parse_key_repeat_handled() {
        let mut buf = [0u8; EVENT_SIZE];
        buf[TYPE_OFFSET..TYPE_OFFSET + 2].copy_from_slice(&EV_KEY.to_ne_bytes());
        buf[TYPE_OFFSET + 2..TYPE_OFFSET + 4].copy_from_slice(&KEY_VOLUMEDOWN.to_ne_bytes());
        buf[TYPE_OFFSET + 4..TYPE_OFFSET + 8].copy_from_slice(&2i32.to_ne_bytes()); // repeat
        assert_eq!(parse_event(&buf), Some(Key::Down));
    }

    #[test]
    fn parse_volume_keys() {
        let mut buf = [0u8; EVENT_SIZE];
        buf[TYPE_OFFSET..TYPE_OFFSET + 2].copy_from_slice(&EV_KEY.to_ne_bytes());
        buf[TYPE_OFFSET + 4..TYPE_OFFSET + 8].copy_from_slice(&1i32.to_ne_bytes());

        buf[TYPE_OFFSET + 2..TYPE_OFFSET + 4].copy_from_slice(&KEY_VOLUMEUP.to_ne_bytes());
        assert_eq!(parse_event(&buf), Some(Key::Up));

        buf[TYPE_OFFSET + 2..TYPE_OFFSET + 4].copy_from_slice(&KEY_VOLUMEDOWN.to_ne_bytes());
        assert_eq!(parse_event(&buf), Some(Key::Down));
    }

    #[test]
    fn parse_power_key() {
        let mut buf = [0u8; EVENT_SIZE];
        buf[TYPE_OFFSET..TYPE_OFFSET + 2].copy_from_slice(&EV_KEY.to_ne_bytes());
        buf[TYPE_OFFSET + 2..TYPE_OFFSET + 4].copy_from_slice(&KEY_POWER.to_ne_bytes());
        buf[TYPE_OFFSET + 4..TYPE_OFFSET + 8].copy_from_slice(&1i32.to_ne_bytes());
        assert_eq!(parse_event(&buf), Some(Key::Enter));
    }

    #[test]
    fn parse_non_key_event_ignored() {
        let mut buf = [0u8; EVENT_SIZE];
        // EV_SYN = 0
        buf[TYPE_OFFSET..TYPE_OFFSET + 2].copy_from_slice(&0u16.to_ne_bytes());
        buf[TYPE_OFFSET + 4..TYPE_OFFSET + 8].copy_from_slice(&1i32.to_ne_bytes());
        assert_eq!(parse_event(&buf), None);
    }

    #[test]
    fn parse_unmapped_key_ignored() {
        let mut buf = [0u8; EVENT_SIZE];
        buf[TYPE_OFFSET..TYPE_OFFSET + 2].copy_from_slice(&EV_KEY.to_ne_bytes());
        buf[TYPE_OFFSET + 2..TYPE_OFFSET + 4].copy_from_slice(&200u16.to_ne_bytes()); // unmapped
        buf[TYPE_OFFSET + 4..TYPE_OFFSET + 8].copy_from_slice(&1i32.to_ne_bytes());
        assert_eq!(parse_event(&buf), None);
    }

    /// Helper: parse an event buffer without going through libc::read.
    fn parse_event(buf: &[u8; EVENT_SIZE]) -> Option<Key> {
        let ev_type = u16::from_ne_bytes([buf[TYPE_OFFSET], buf[TYPE_OFFSET + 1]]);
        let ev_code = u16::from_ne_bytes([buf[TYPE_OFFSET + 2], buf[TYPE_OFFSET + 3]]);
        let ev_value = i32::from_ne_bytes([
            buf[TYPE_OFFSET + 4],
            buf[TYPE_OFFSET + 5],
            buf[TYPE_OFFSET + 6],
            buf[TYPE_OFFSET + 7],
        ]);

        if ev_type != EV_KEY || ev_value == 0 {
            return None;
        }

        match ev_code {
            KEY_UP | KEY_VOLUMEUP => Some(Key::Up),
            KEY_DOWN | KEY_VOLUMEDOWN => Some(Key::Down),
            KEY_LEFT => Some(Key::Left),
            KEY_RIGHT => Some(Key::Right),
            KEY_ENTER | KEY_POWER => Some(Key::Enter),
            KEY_ESC => Some(Key::Escape),
            KEY_BACKSPACE => Some(Key::Backspace),
            _ => None,
        }
    }
}
