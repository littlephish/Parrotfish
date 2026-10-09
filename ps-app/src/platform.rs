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
        seal_for(PURPOSE, data, open)
    }

    fn seal_for(purpose: &[u8], data: &[u8], open: bool) -> Option<Vec<u8>> {
        let size = u32::try_from(data.len()).ok().filter(|size| *size > 0)?;
        let input = Blob { size, data: data.as_ptr() as *mut u8 };
        let purpose = Blob { size: purpose.len() as u32, data: purpose.as_ptr() as *mut u8 };
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

    #[link(name = "winhttp")]
    extern "system" {
        fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> isize;
        fn WinHttpConnect(session: isize, host: *const u16, port: u16, reserved: u32) -> isize;
        fn WinHttpOpenRequest(
            connection: isize,
            verb: *const u16,
            path: *const u16,
            version: *const u16,
            referrer: *const u16,
            accept: *const *const u16,
            flags: u32,
        ) -> isize;
        fn WinHttpSetOption(handle: isize, option: u32, value: *const core::ffi::c_void, size: u32) -> i32;
        fn WinHttpSetTimeouts(handle: isize, resolve: i32, connect: i32, send: i32, receive: i32) -> i32;
        fn WinHttpSendRequest(
            request: isize,
            headers: *const u16,
            headers_size: u32,
            body: *const core::ffi::c_void,
            body_size: u32,
            total_size: u32,
            context: usize,
        ) -> i32;
        fn WinHttpReceiveResponse(request: isize, reserved: *mut core::ffi::c_void) -> i32;
        fn WinHttpQueryHeaders(
            request: isize,
            what: u32,
            name: *const u16,
            value: *mut core::ffi::c_void,
            size: *mut u32,
            index: *mut u32,
        ) -> i32;
        fn WinHttpReadData(request: isize, into: *mut core::ffi::c_void, size: u32, read: *mut u32) -> i32;
        fn WinHttpCloseHandle(handle: isize) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLastError() -> u32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            owner: isize,
            verb: *const u16,
            file: *const u16,
            arguments: *const u16,
            folder: *const u16,
            show: i32,
        ) -> isize;
    }

    const SYSTEM_PROXY: u32 = 4;
    const USUAL_PROXY: u32 = 0;
    const ENCRYPTED: u32 = 0x0080_0000;
    const DISABLE_FEATURE: u32 = 63;
    const COOKIES_REDIRECTS_AND_SIGNING_IN: u32 = 1 | 2 | 4;
    const STATUS_AS_NUMBER: u32 = 19 | 0x2000_0000;
    const LENGTH_AS_NUMBER: u32 = 5 | 0x2000_0000;
    const LOCATION: u32 = 33;
    const WEB_WAIT_MS: i32 = 20_000;
    const WEB_CHUNK: usize = 64 * 1024;
    const SHOW: i32 = 1;

    struct Web(isize);

    impl Drop for Web {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    fn web_problem(step: &str) -> String {
        match unsafe { GetLastError() } {
            12002 => "the server took too long to answer".to_string(),
            12007 => "the server's name could not be found".to_string(),
            12029..=12031 => "the connection could not be made, or was cut".to_string(),
            12037 | 12038 | 12045 | 12157 | 12169 | 12175 => "the secure connection could not be trusted".to_string(),
            code => format!("Windows error {code} while {step}"),
        }
    }

    fn number_header(request: isize, what: u32) -> Option<u32> {
        let mut value = 0u32;
        let mut size = 4u32;
        let read = unsafe {
            WinHttpQueryHeaders(request, what, std::ptr::null(), (&mut value as *mut u32).cast(), &mut size, std::ptr::null_mut())
        };
        (read != 0).then_some(value)
    }

    fn text_header(request: isize, what: u32) -> Option<String> {
        let mut size = 0u32;
        unsafe { WinHttpQueryHeaders(request, what, std::ptr::null(), std::ptr::null_mut(), &mut size, std::ptr::null_mut()) };
        if size == 0 || size > 16_384 {
            return None;
        }
        let mut text = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut size = ((text.len() - 1) * 2) as u32;
        let read = unsafe {
            WinHttpQueryHeaders(request, what, std::ptr::null(), text.as_mut_ptr().cast(), &mut size, std::ptr::null_mut())
        };
        if read == 0 {
            return None;
        }
        let end = text.iter().position(|unit| *unit == 0).unwrap_or(text.len());
        Some(String::from_utf16_lossy(&text[..end]))
    }

    pub fn web_get(
        url: &str,
        agent: &str,
        limit: usize,
        progress: &mut dyn FnMut(usize, Option<u64>),
    ) -> Result<super::WebReply, String> {
        let address = super::web_address(url).ok_or("that is not an address this program fetches")?;
        let agent = wide(agent);
        let mut session = Web(unsafe { WinHttpOpen(agent.as_ptr(), SYSTEM_PROXY, std::ptr::null(), std::ptr::null(), 0) });
        if session.0 == 0 {
            session = Web(unsafe { WinHttpOpen(agent.as_ptr(), USUAL_PROXY, std::ptr::null(), std::ptr::null(), 0) });
        }
        if session.0 == 0 {
            return Err(web_problem("starting"));
        }
        unsafe { WinHttpSetTimeouts(session.0, WEB_WAIT_MS, WEB_WAIT_MS, WEB_WAIT_MS, WEB_WAIT_MS) };
        let host = wide(&address.host);
        let connection = Web(unsafe { WinHttpConnect(session.0, host.as_ptr(), address.port, 0) });
        if connection.0 == 0 {
            return Err(web_problem("connecting"));
        }
        let (verb, path) = (wide("GET"), wide(&address.path));
        let flags = if address.secure { ENCRYPTED } else { 0 };
        let request = Web(unsafe {
            WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                flags,
            )
        });
        if request.0 == 0 {
            return Err(web_problem("asking"));
        }
        let off = COOKIES_REDIRECTS_AND_SIGNING_IN;
        if unsafe { WinHttpSetOption(request.0, DISABLE_FEATURE, (&off as *const u32).cast(), 4) } == 0 {
            return Err(web_problem("asking"));
        }
        let sent = unsafe { WinHttpSendRequest(request.0, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) } != 0;
        if !sent || unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) } == 0 {
            return Err(web_problem("asking"));
        }
        let status = number_header(request.0, STATUS_AS_NUMBER).ok_or("the server's answer could not be read")?;
        let location = text_header(request.0, LOCATION).unwrap_or_default();
        let length = number_header(request.0, LENGTH_AS_NUMBER).map(u64::from);
        let mut body = Vec::new();
        if status == 200 {
            if length.is_some_and(|length| length > limit as u64) {
                return Err("the file is larger than it should be".to_string());
            }
            let mut chunk = vec![0u8; WEB_CHUNK];
            loop {
                let mut read = 0u32;
                if unsafe { WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read) } == 0 {
                    return Err(web_problem("downloading"));
                }
                if read == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..read as usize]);
                if body.len() > limit {
                    return Err("the file is larger than it should be".to_string());
                }
                progress(body.len(), length);
            }
            if let Some(length) = length.filter(|length| *length != body.len() as u64) {
                return Err(format!("the download stopped early: {} of {length} bytes arrived", body.len()));
            }
        }
        Ok(super::WebReply { status, location, length, body })
    }

    pub fn open_link(url: &str) -> bool {
        if !url.starts_with("https://") {
            return false;
        }
        let (verb, file) = (wide("open"), wide(url));
        unsafe { ShellExecuteW(0, verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SHOW) > 32 }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        fn serve(answer: Vec<u8>) -> String {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            std::thread::spawn(move || {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut asked = Vec::new();
                    let mut piece = [0u8; 1024];
                    while !asked.windows(4).any(|part| part == b"\r\n\r\n") {
                        match stream.read(&mut piece) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => asked.extend_from_slice(&piece[..n]),
                        }
                    }
                    let _ = stream.write_all(&answer);
                }
            });
            format!("http://127.0.0.1:{port}/files/thing.zip")
        }

        fn get(url: &str, limit: usize) -> (Result<crate::platform::WebReply, String>, Vec<(usize, Option<u64>)>) {
            let mut seen = Vec::new();
            let reply = web_get(url, "Parrotfish-test", limit, &mut |so_far, of| seen.push((so_far, of)));
            (reply, seen)
        }

        #[test]
        fn a_file_is_fetched_whole_and_its_progress_reported() {
            let mut answer = b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n".to_vec();
            answer.extend(std::iter::repeat(b'x').take(5000));
            let (reply, seen) = get(&serve(answer), 10_000);
            let reply = reply.expect("a complete answer");
            assert_eq!((reply.status, reply.length, reply.body.len()), (200, Some(5000), 5000));
            assert_eq!(seen.last(), Some(&(5000, Some(5000))));
        }

        #[test]
        fn a_download_that_stops_early_is_not_taken_for_the_file() {
            let mut answer = b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n".to_vec();
            answer.extend(std::iter::repeat(b'x').take(400));
            let (reply, _) = get(&serve(answer), 10_000);
            assert!(reply.is_err(), "400 of 5000 bytes were accepted as the whole file");
        }

        #[test]
        fn a_file_larger_than_allowed_is_refused() {
            let mut answer = b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n".to_vec();
            answer.extend(std::iter::repeat(b'x').take(5000));
            let (reply, _) = get(&serve(answer), 4_999);
            assert_eq!(reply, Err("the file is larger than it should be".to_string()));
            let mut unsized_answer = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
            unsized_answer.extend(std::iter::repeat(b'x').take(5000));
            let (reply, _) = get(&serve(unsized_answer.clone()), 4_999);
            assert_eq!(reply, Err("the file is larger than it should be".to_string()));
            let (reply, _) = get(&serve(unsized_answer), 5_000);
            assert_eq!(reply.map(|reply| (reply.length, reply.body.len())), Ok((None, 5000)));
        }

        #[test]
        fn a_redirect_is_reported_and_not_followed() {
            let answer = b"HTTP/1.1 302 Found\r\nLocation: https://elsewhere.example/next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
            let (reply, seen) = get(&serve(answer), 10_000);
            let reply = reply.expect("the redirect itself is an answer");
            assert_eq!((reply.status, reply.location.as_str(), reply.body.len()), (302, "https://elsewhere.example/next", 0));
            assert!(seen.is_empty());
            let missing = b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot here.".to_vec();
            let (reply, _) = get(&serve(missing), 10_000);
            assert_eq!(reply.map(|reply| (reply.status, reply.body.len())), Ok((404, 0)));
        }

        #[test]
        fn only_secure_addresses_are_opened_in_the_browser() {
            assert!(!open_link("http://example.org/"));
            assert!(!open_link("file:///C:/Windows/System32/calc.exe"));
            assert!(!open_link("calc.exe"));
        }

        #[test]
        fn a_password_sealed_before_the_program_was_renamed_still_opens() {
            let earlier = seal_for(b"PhishSpeak bookmark", b"reef-pass", false).expect("Windows seals it");
            assert_eq!(unprotect(&earlier).as_deref(), Some(&b"reef-pass"[..]));
            let stranger = seal_for(b"Parrotfish bookmark", b"reef-pass", false).expect("Windows seals it");
            assert_eq!(unprotect(&stranger), None, "sealed for another purpose, it must not open");
            let own = protect(b"reef-pass").expect("Windows seals it");
            assert!(!own.windows(9).any(|part| part == b"reef-pass"));
            assert_eq!(seal_for(b"PhishSpeak bookmark", &own, true).as_deref(), Some(&b"reef-pass"[..]));
            assert_eq!(protect(b""), None);
        }
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

    pub fn web_get(
        _url: &str,
        _agent: &str,
        _limit: usize,
        _progress: &mut dyn FnMut(usize, Option<u64>),
    ) -> Result<super::WebReply, String> {
        Err("this build cannot fetch anything".to_string())
    }

    pub fn open_link(_url: &str) -> bool {
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

pub use imp::{
    allow_front, bring_front, clear_link_handler, key_char, key_down, link_handler, local_hms, on_a_screen, open_link,
    overlay_style, own_front_window, protect, set_link_handler, show_own_window, unprotect, web_get,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebReply {
    pub status: u32,
    pub location: String,
    pub length: Option<u64>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebAddress {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn web_address(url: &str) -> Option<WebAddress> {
    let (secure, rest) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(rest), _) => (true, rest),
        (None, Some(rest)) if cfg!(test) => (false, rest),
        _ => return None,
    };
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok().filter(|port| *port != 0)?),
        None => (authority, if secure { 443 } else { 80 }),
    };
    let named = !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let plain = !path.chars().any(|c| c.is_control() || c.is_whitespace() || c == '#');
    let allowed = secure || host == "127.0.0.1";
    (named && plain && allowed).then(|| WebAddress { secure, host: host.to_ascii_lowercase(), port, path: path.to_string() })
}

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
    fn only_plain_encrypted_addresses_are_fetched() {
        let page = web_address("https://github.com/littlephish/Parrotfish/releases/latest?x=1").expect("a usual address");
        assert_eq!((page.secure, page.host.as_str(), page.port), (true, "github.com", 443));
        assert_eq!(page.path, "/littlephish/Parrotfish/releases/latest?x=1");
        assert_eq!(web_address("https://GitHub.com").map(|bare| (bare.host, bare.path)), Some(("github.com".to_string(), "/".to_string())));
        assert_eq!(web_address("https://files.example:8443/a").map(|other| other.port), Some(8443));
        assert!(web_address("http://127.0.0.1:8080/x").is_some_and(|local| !local.secure), "tests may talk to this PC unencrypted");
        for refused in [
            "http://github.com/x",
            "http://localhost/x",
            "ftp://github.com/x",
            "github.com/x",
            "https://",
            "https:///x",
            "https://user@github.com/x",
            "https://github.com:0/x",
            "https://github.com:99999/x",
            "https://github.com:port/x",
            "https://git hub.com/x",
            "https://github.com/a b",
            "https://github.com/a#b",
            "https://github.com/a\r\nHost: evil.example",
            "",
        ] {
            assert_eq!(web_address(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn keys_that_do_not_exist_are_never_down() {
        assert!(!key_down(0));
        assert!(!key_down(-5));
        assert_eq!(crate::hotkeys::key_name(0x41, &key_char), if cfg!(windows) { "A" } else { "Key 65" });
    }
}
