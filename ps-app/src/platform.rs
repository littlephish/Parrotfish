pub const PTT_KEYS: &[(&str, i32)] = &[
    ("No hotkey", 0),
    ("Left Ctrl", 0xA2),
    ("Right Ctrl", 0xA3),
    ("Left Alt", 0xA4),
    ("Right Alt", 0xA5),
    ("Left Shift", 0xA0),
    ("Right Shift", 0xA1),
    ("Caps Lock", 0x14),
    ("Mouse 4", 0x05),
    ("Mouse 5", 0x06),
    ("Middle mouse", 0x04),
    ("` (backtick)", 0xC0),
    ("Scroll Lock", 0x91),
    ("Pause", 0x13),
    ("F8", 0x77),
    ("F9", 0x78),
    ("F10", 0x79),
];

#[cfg(windows)]
mod imp {
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(time: *mut SystemTime);
    }

    pub fn key_down(vkey: i32) -> bool {
        if vkey <= 0 {
            return false;
        }
        let state = unsafe { GetAsyncKeyState(vkey) };
        (state as u16) & 0x8000 != 0
    }

    pub fn local_hms() -> (u32, u32, u32) {
        let mut t = SystemTime::default();
        unsafe { GetLocalTime(&mut t) };
        (t.hour as u32, t.minute as u32, t.second as u32)
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn key_down(_vkey: i32) -> bool {
        false
    }

    pub fn local_hms() -> (u32, u32, u32) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        (((secs / 3600) % 24) as u32, ((secs / 60) % 60) as u32, (secs % 60) as u32)
    }
}

pub use imp::{key_down, local_hms};

pub fn timestamp() -> String {
    let (h, m, s) = local_hms();
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_is_well_formed() {
        let t = timestamp();
        assert_eq!(t.len(), 8);
        let parts: Vec<u32> = t.split(':').map(|p| p.parse().unwrap()).collect();
        assert!(parts[0] < 24 && parts[1] < 60 && parts[2] < 61);
    }

    #[test]
    fn hotkey_table_is_sane() {
        assert_eq!(PTT_KEYS[0].1, 0);
        assert!(!key_down(0));
        assert!(!key_down(-5));
        let mut codes: Vec<i32> = PTT_KEYS.iter().map(|k| k.1).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), PTT_KEYS.len());
    }
}
