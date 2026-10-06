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
        fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
    }

    pub fn key_char(vk: u16) -> Option<char> {
        let mapped = unsafe { MapVirtualKeyW(u32::from(vk), 2) } & 0xFFFF;
        if mapped == 0 {
            None
        } else {
            char::from_u32(mapped)
        }
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

    pub fn key_char(_vk: u16) -> Option<char> {
        None
    }

    pub fn local_hms() -> (u32, u32, u32) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        (((secs / 3600) % 24) as u32, ((secs / 60) % 60) as u32, (secs % 60) as u32)
    }
}

pub use imp::{key_char, key_down, local_hms};

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
    fn keys_that_do_not_exist_are_never_down() {
        assert!(!key_down(0));
        assert!(!key_down(-5));
        assert_eq!(crate::hotkeys::key_name(0x41, &key_char), if cfg!(windows) { "A" } else { "Key 65" });
    }
}
