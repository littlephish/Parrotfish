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

    #[repr(C)]
    struct Blob {
        size: u32,
        data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut core::ffi::c_void,
            prompt: *mut core::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut core::ffi::c_void,
            prompt: *mut core::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    const NO_PROMPT: u32 = 1;
    const PURPOSE: &[u8] = b"PhishSpeak bookmark";

    fn seal(data: &[u8], open: bool) -> Option<Vec<u8>> {
        let size = u32::try_from(data.len()).ok().filter(|size| *size > 0)?;
        let input = Blob { size, data: data.as_ptr() as *mut u8 };
        let purpose = Blob { size: PURPOSE.len() as u32, data: PURPOSE.as_ptr() as *mut u8 };
        let mut output = Blob { size: 0, data: std::ptr::null_mut() };
        let nothing = std::ptr::null_mut();
        let done = unsafe {
            if open {
                CryptUnprotectData(&input, std::ptr::null_mut(), &purpose, nothing, nothing, NO_PROMPT, &mut output)
            } else {
                CryptProtectData(&input, std::ptr::null(), &purpose, nothing, nothing, NO_PROMPT, &mut output)
            }
        };
        if done == 0 || output.data.is_null() {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(output.data, output.size as usize) }.to_vec();
        unsafe { LocalFree(output.data.cast()) };
        Some(bytes)
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowExW(parent: isize, after: isize, class: *const u16, title: *const u16) -> isize;
        fn GetWindowThreadProcessId(window: isize, process: *mut u32) -> u32;
        fn GetWindowLongPtrW(window: isize, index: i32) -> isize;
        fn SetWindowLongPtrW(window: isize, index: i32, value: isize) -> isize;
        fn SetLayeredWindowAttributes(window: isize, key: u32, alpha: u8, flags: u32) -> i32;
        fn MonitorFromPoint(point: Point, flags: u32) -> isize;
        fn GetForegroundWindow() -> isize;
        fn SetForegroundWindow(window: isize) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
    }

    const EXTENDED_STYLE: i32 = -20;
    const LAYERED: isize = 0x0008_0000;
    const PASS_CLICKS: isize = 0x0000_0020;
    const NEVER_ACTIVE: isize = 0x0800_0000;
    const WHOLE_WINDOW_ALPHA: u32 = 2;
    const SAME_TITLE_LIMIT: usize = 64;

    fn own_window(title: &str) -> Option<isize> {
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let me = unsafe { GetCurrentProcessId() };
        let mut window = 0isize;
        for _ in 0..SAME_TITLE_LIMIT {
            window = unsafe { FindWindowExW(0, window, std::ptr::null(), wide.as_ptr()) };
            if window == 0 {
                return None;
            }
            let mut owner = 0u32;
            unsafe { GetWindowThreadProcessId(window, &mut owner) };
            if owner == me {
                return Some(window);
            }
        }
        None
    }

    pub fn overlay_style(title: &str, alpha: u8, pass_clicks: bool) -> bool {
        let Some(window) = own_window(title) else {
            return false;
        };
        let style = unsafe { GetWindowLongPtrW(window, EXTENDED_STYLE) };
        let plain = style | LAYERED | NEVER_ACTIVE;
        let wanted = if pass_clicks { plain | PASS_CLICKS } else { plain & !PASS_CLICKS };
        if wanted != style {
            unsafe { SetWindowLongPtrW(window, EXTENDED_STYLE, wanted) };
        }
        unsafe { SetLayeredWindowAttributes(window, 0, alpha, WHOLE_WINDOW_ALPHA) != 0 }
    }

    pub fn on_a_screen(x: i32, y: i32) -> bool {
        unsafe { MonitorFromPoint(Point { x, y }, 0) != 0 }
    }

    pub fn own_front_window() -> isize {
        let window = unsafe { GetForegroundWindow() };
        if window == 0 {
            return 0;
        }
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut owner) };
        if owner == unsafe { GetCurrentProcessId() } {
            window
        } else {
            0
        }
    }

    pub fn bring_front(window: isize) {
        if window != 0 && window != unsafe { GetForegroundWindow() } {
            unsafe { SetForegroundWindow(window) };
        }
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(
            key: isize,
            sub: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            access: u32,
            security: *const core::ffi::c_void,
            result: *mut isize,
            disposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(key: isize, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
        fn RegGetValueW(
            key: isize,
            sub: *const u16,
            name: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut core::ffi::c_void,
            size: *mut u32,
        ) -> i32;
        fn RegDeleteTreeW(key: isize, sub: *const u16) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHChangeNotify(event: i32, flags: u32, first: *const core::ffi::c_void, second: *const core::ffi::c_void);
    }

    #[link(name = "user32")]
    extern "system" {
        fn AllowSetForegroundWindow(process: u32) -> i32;
        fn ShowWindow(window: isize, command: i32) -> i32;
        fn IsIconic(window: isize) -> i32;
    }

    const CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const MAY_WRITE: u32 = 0x0002_0006;
    const PLAIN_TEXT: u32 = 1;
    const ANY_TEXT_AS_STORED: u32 = 0x1000_0006;
    const NOT_THERE: i32 = 2;
    const ASSOCIATIONS_CHANGED: i32 = 0x0800_0000;
    const RESTORE: i32 = 9;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn scheme_key(scheme: &str) -> String {
        format!("Software\\Classes\\{scheme}")
    }

    fn set_text(path: &str, name: Option<&str>, value: &str) -> bool {
        let path = wide(path);
        let mut key = 0isize;
        let opened = unsafe {
            RegCreateKeyExW(
                CURRENT_USER,
                path.as_ptr(),
                0,
                std::ptr::null(),
                0,
                MAY_WRITE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };
        if opened != 0 {
            return false;
        }
        let name = name.map(wide);
        let name_ptr = name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr());
        let data = wide(value);
        let size = (data.len() * 2) as u32;
        let done = unsafe { RegSetValueExW(key, name_ptr, 0, PLAIN_TEXT, data.as_ptr().cast(), size) } == 0;
        unsafe { RegCloseKey(key) };
        done
    }

    fn tell_the_shell() {
        unsafe { SHChangeNotify(ASSOCIATIONS_CHANGED, 0, std::ptr::null(), std::ptr::null()) };
    }

    pub fn link_handler(scheme: &str) -> Option<String> {
        let path = wide(&format!("{}\\shell\\open\\command", scheme_key(scheme)));
        let mut size = 0u32;
        let asked = unsafe {
            RegGetValueW(
                CURRENT_USER,
                path.as_ptr(),
                std::ptr::null(),
                ANY_TEXT_AS_STORED,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if asked != 0 || size < 2 || size > 65_536 {
            return None;
        }
        let mut data = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut size = ((data.len() - 1) * 2) as u32;
        let read = unsafe {
            RegGetValueW(
                CURRENT_USER,
                path.as_ptr(),
                std::ptr::null(),
                ANY_TEXT_AS_STORED,
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if read != 0 {
            return None;
        }
        let end = data.iter().position(|unit| *unit == 0).unwrap_or(data.len());
        let text = String::from_utf16_lossy(&data[..end]);
        (!text.trim().is_empty()).then_some(text)
    }

    pub fn set_link_handler(scheme: &str, command: &str) -> bool {
        let base = scheme_key(scheme);
        let done = set_text(&base, None, &format!("URL:{scheme} link"))
            && set_text(&base, Some("URL Protocol"), "")
            && set_text(&format!("{base}\\shell\\open\\command"), None, command);
        tell_the_shell();
        done
    }

    pub fn clear_link_handler(scheme: &str) -> bool {
        let path = wide(&scheme_key(scheme));
        let code = unsafe { RegDeleteTreeW(CURRENT_USER, path.as_ptr()) };
        tell_the_shell();
        code == 0 || code == NOT_THERE
    }

    pub fn allow_front(process: u32) {
        unsafe { AllowSetForegroundWindow(process) };
    }

    pub fn show_own_window(title: &str) -> bool {
        let Some(window) = own_window(title) else {
            return false;
        };
        if unsafe { IsIconic(window) } != 0 {
            unsafe { ShowWindow(window, RESTORE) };
        }
        unsafe { SetForegroundWindow(window) != 0 }
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        seal(data, false)
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        seal(data, true)
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

    pub fn overlay_style(_title: &str, _alpha: u8, _pass_clicks: bool) -> bool {
        false
    }

    pub fn on_a_screen(_x: i32, _y: i32) -> bool {
        true
    }

    pub fn own_front_window() -> isize {
        0
    }

    pub fn bring_front(_window: isize) {}

    pub fn link_handler(_scheme: &str) -> Option<String> {
        None
    }

    pub fn set_link_handler(_scheme: &str, _command: &str) -> bool {
        false
    }

    pub fn clear_link_handler(_scheme: &str) -> bool {
        false
    }

    pub fn allow_front(_process: u32) {}

    pub fn show_own_window(_title: &str) -> bool {
        false
    }

    pub fn protect(_data: &[u8]) -> Option<Vec<u8>> {
        None
    }

    pub fn unprotect(_data: &[u8]) -> Option<Vec<u8>> {
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

pub use imp::{
    allow_front, bring_front, clear_link_handler, key_char, key_down, link_handler, local_hms, on_a_screen,
    overlay_style, own_front_window, protect, set_link_handler, show_own_window, unprotect,
};

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
