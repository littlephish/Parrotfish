#[cfg(not(windows))]
fn main() {
    println!("scaledrag is for Windows: it watches a window cross displays with different scales.");
}

#[cfg(windows)]
#[path = "../src/scale.rs"]
#[allow(dead_code)]
mod scale;

#[cfg(windows)]
#[path = "../src/platform.rs"]
#[allow(dead_code, unused_imports)]
mod platform;

#[cfg(windows)]
fn main() {
    harness::run();
}

#[cfg(windows)]
mod harness {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicI64, AtomicIsize, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use slint::{ComponentHandle, LogicalSize, Timer, TimerMode};

    use crate::scale::ScaleWatch;

    slint::slint! {
        export component Harness inherits Window {
            in property <float> scale-nudge;
            in property <string> note;
            title: "Parrotfish scale harness";
            preferred-width: 400px;
            preferred-height: 740px;
            min-width: 340px + root.scale-nudge * 0.01px;
            min-height: 520px;
            background: #12303a;
            Text {
                x: 12px;
                y: 12px;
                width: parent.width - 24px;
                text: root.note;
                color: white;
                wrap: word-wrap;
                font-size: 14px;
            }
        }
    }

    const TITLE: &str = "Parrotfish scale harness";

    #[repr(C)]
    #[derive(Clone, Copy, Default, Debug, PartialEq)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct TrackLimits {
        reserved: Point,
        largest: Point,
        largest_at: Point,
        least: Point,
        most: Point,
    }

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowExW(parent: isize, after: isize, class: *const u16, title: *const u16) -> isize;
        fn GetWindowThreadProcessId(window: isize, process: *mut u32) -> u32;
        fn GetWindowRect(window: isize, rect: *mut Rect) -> i32;
        fn GetClientRect(window: isize, rect: *mut Rect) -> i32;
        fn GetDpiForWindow(window: isize) -> u32;
        fn SetWindowPos(window: isize, after: isize, x: i32, y: i32, wide: i32, high: i32, flags: u32) -> i32;
        fn GetForegroundWindow() -> isize;
        fn SetForegroundWindow(window: isize) -> i32;
        fn GetWindowLongPtrW(window: isize, index: i32) -> isize;
        fn SetWindowLongPtrW(window: isize, index: i32, value: isize) -> isize;
        fn CallWindowProcW(previous: isize, window: isize, message: u32, first: usize, second: isize) -> isize;
        fn GetCursorPos(point: *mut Point) -> i32;
        fn SetCursorPos(x: i32, y: i32) -> i32;
        fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
        fn GetSystemMetrics(index: i32) -> i32;
        fn WindowFromPoint(point: Point) -> isize;
        fn GetAncestor(window: isize, which: u32) -> isize;
    }

    #[repr(C)]
    struct MouseInput {
        x: i32,
        y: i32,
        data: u32,
        flags: u32,
        time: u32,
        extra: usize,
    }

    #[repr(C)]
    struct Input {
        kind: u32,
        mouse: MouseInput,
    }

    const MOUSE_MOVES: u32 = 0x0001;
    const LEFT_DOWN: u32 = 0x0002;
    const LEFT_UP: u32 = 0x0004;
    const WHOLE_DESKTOP: u32 = 0x4000;
    const ABSOLUTE: u32 = 0x8000;
    const ROOT: u32 = 2;
    const ABOVE_ALL: isize = -1;
    const KEEP_PLACE: u32 = 0x0002;

    fn mouse(flags: u32, x: i32, y: i32) {
        let (left, top, wide, high) = unsafe { (GetSystemMetrics(76), GetSystemMetrics(77), GetSystemMetrics(78), GetSystemMetrics(79)) };
        let across = (f64::from(x - left) * 65535.0 / f64::from((wide - 1).max(1))).round() as i32;
        let down = (f64::from(y - top) * 65535.0 / f64::from((high - 1).max(1))).round() as i32;
        let input = Input { kind: 0, mouse: MouseInput { x: across, y: down, data: 0, flags: flags | ABSOLUTE | WHOLE_DESKTOP, time: 0, extra: 0 } };
        unsafe { SendInput(1, &input, std::mem::size_of::<Input>() as i32) };
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
    }

    const WINDOW_PROCEDURE: i32 = -4;
    const KEEP_SIZE: u32 = 0x0001;
    const KEEP_ORDER: u32 = 0x0004;
    const STAY_BACK: u32 = 0x0010;
    const LIMITS_ASKED: u32 = 0x0024;
    const PLACED: u32 = 0x0047;
    const MOVE_BEGINS: u32 = 0x0231;
    const MOVE_ENDS: u32 = 0x0232;
    const SCALE_CHANGED: u32 = 0x02E0;
    const PLACING: u32 = 0x0046;
    const SCALED_SIZE_ASKED: u32 = 0x02E4;
    const NO_MOVE: u32 = 0x0002;

    #[repr(C)]
    struct Placing {
        window: isize,
        after: isize,
        x: i32,
        y: i32,
        wide: i32,
        high: i32,
        flags: u32,
    }

    static DEPTH: AtomicI64 = AtomicI64::new(0);
    static DRAGGED: AtomicI64 = AtomicI64::new(0);
    static FIX: AtomicI64 = AtomicI64::new(0);
    static SUGGESTED: Mutex<Vec<Rect>> = Mutex::new(Vec::new());

    static STARTED: OnceLock<Instant> = OnceLock::new();
    static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static WINDOW: AtomicIsize = AtomicIsize::new(0);
    static PREVIOUS: AtomicIsize = AtomicIsize::new(0);
    static LAST_INNER: AtomicI64 = AtomicI64::new(-1);
    static LAST_LEAST: AtomicI64 = AtomicI64::new(-1);

    fn note(text: String) {
        let at = STARTED.get_or_init(Instant::now).elapsed().as_millis();
        if let Ok(mut lines) = LINES.lock() {
            lines.push(format!("{at:>6} ms  {text}"));
        }
    }

    fn pair(a: i32, b: i32) -> i64 {
        (i64::from(a) << 32) | i64::from(b as u32)
    }

    fn outer(window: isize) -> Rect {
        let mut rect = Rect::default();
        unsafe { GetWindowRect(window, &mut rect) };
        rect
    }

    fn inner(window: isize) -> (i32, i32) {
        let mut rect = Rect::default();
        unsafe { GetClientRect(window, &mut rect) };
        (rect.right, rect.bottom)
    }

    fn pointer() -> Point {
        let mut point = Point::default();
        unsafe { GetCursorPos(&mut point) };
        point
    }

    fn state(window: isize) -> String {
        let rect = outer(window);
        let (wide, high) = inner(window);
        let dpi = unsafe { GetDpiForWindow(window) };
        format!(
            "at ({},{}) outer {}x{} inner {}x{} dpi {} => {:.1}x{:.1} logical",
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            wide,
            high,
            dpi,
            f64::from(wide) * 96.0 / f64::from(dpi.max(1)),
            f64::from(high) * 96.0 / f64::from(dpi.max(1))
        )
    }

    unsafe extern "system" fn watching(window: isize, message: u32, first: usize, second: isize) -> isize {
        let previous = PREVIOUS.load(Ordering::Relaxed);
        match message {
            SCALE_CHANGED => {
                let dpi = first & 0xffff;
                let suggested = unsafe { *(second as *const Rect) };
                let pointer = pointer();
                note(format!(
                    "SCALE CHANGE to dpi {dpi}, Windows suggests ({},{}) {}x{} | pointer ({},{}) | before: {}",
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    pointer.x,
                    pointer.y,
                    state(window)
                ));
                if let Ok(mut suggestions) = SUGGESTED.lock() {
                    suggestions.push(suggested);
                }
                DEPTH.fetch_add(1, Ordering::Relaxed);
                let result = unsafe { CallWindowProcW(previous, window, message, first, second) };
                DEPTH.fetch_sub(1, Ordering::Relaxed);
                if let Ok(mut suggestions) = SUGGESTED.lock() {
                    suggestions.pop();
                }
                note(format!("    handled (depth now {}) | after: {}", DEPTH.load(Ordering::Relaxed), state(window)));
                result
            }
            PLACING if second != 0 && DEPTH.load(Ordering::Relaxed) > 0 => {
                let placing = unsafe { &mut *(second as *mut Placing) };
                let whole = placing.flags & (KEEP_SIZE | NO_MOVE) == 0;
                note(format!(
                    "    the toolkit asks for ({},{}) {}x{} flags {:#06x} at depth {}",
                    placing.x,
                    placing.y,
                    placing.wide,
                    placing.high,
                    placing.flags,
                    DEPTH.load(Ordering::Relaxed)
                ));
                let asked = (placing.x, placing.y, placing.wide, placing.high);
                let result = unsafe { CallWindowProcW(previous, window, message, first, second) };
                let placing = unsafe { &*(second as *const Placing) };
                if whole && (placing.x, placing.y, placing.wide, placing.high) != asked {
                    note(format!("    FIX: placed where Windows suggested instead: ({},{}) {}x{}", placing.x, placing.y, placing.wide, placing.high));
                }
                result
            }
            SCALED_SIZE_ASKED => {
                let result = unsafe { CallWindowProcW(previous, window, message, first, second) };
                note(format!("    Windows asks what size the window wants at dpi {first}; the toolkit answers {result}"));
                result
            }
            MOVE_BEGINS | MOVE_ENDS => {
                DRAGGED.store(i64::from(message == MOVE_BEGINS), Ordering::Relaxed);
                note(format!("{} | {}", if message == MOVE_BEGINS { "DRAG BEGINS" } else { "DRAG ENDS" }, state(window)));
                unsafe { CallWindowProcW(previous, window, message, first, second) }
            }
            PLACED => {
                let result = unsafe { CallWindowProcW(previous, window, message, first, second) };
                let (wide, high) = inner(window);
                if LAST_INNER.swap(pair(wide, high), Ordering::Relaxed) != pair(wide, high) {
                    note(format!("    size is now | {}", state(window)));
                } else if DEPTH.load(Ordering::Relaxed) > 0 {
                    note(format!("    placed, same size | {}", state(window)));
                }
                result
            }
            LIMITS_ASKED => {
                let result = unsafe { CallWindowProcW(previous, window, message, first, second) };
                if second != 0 {
                    let least = unsafe { (*(second as *const TrackLimits)).least };
                    if LAST_LEAST.swap(pair(least.x, least.y), Ordering::Relaxed) != pair(least.x, least.y) {
                        let dpi = unsafe { GetDpiForWindow(window) };
                        note(format!("    least outer size answered: {}x{} while dpi {dpi}", least.x, least.y));
                    }
                }
                result
            }
            _ => unsafe { CallWindowProcW(previous, window, message, first, second) },
        }
    }

    fn own_window(title: &str) -> Option<isize> {
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let me = unsafe { GetCurrentProcessId() };
        let mut window = 0isize;
        for _ in 0..64 {
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

    struct Asked {
        mode: String,
        fix: bool,
        follow: bool,
        grab: f64,
        travel: i32,
        from: i32,
        to: i32,
        y: i32,
        step: i32,
        pause: u64,
        passes: u32,
        seconds: u64,
        width: f32,
        height: f32,
        log: String,
    }

    fn asked() -> Asked {
        let mut asked = Asked {
            mode: "hand".to_string(),
            fix: false,
            follow: true,
            grab: 0.3,
            travel: 420,
            from: 3500,
            to: 3780,
            y: 68,
            step: 3,
            pause: 12,
            passes: 2,
            seconds: 90,
            width: 385.33,
            height: 1073.33,
            log: "scaledrag.log".to_string(),
        };
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let mut at = 0;
        while at + 1 < arguments.len() {
            let value = &arguments[at + 1];
            match arguments[at].as_str() {
                "--mode" => asked.mode = value.clone(),
                "--fix" => asked.fix = value == "on",
                "--follow" => asked.follow = value != "off",
                "--grab" => asked.grab = value.parse().unwrap_or(asked.grab),
                "--travel" => asked.travel = value.parse().unwrap_or(asked.travel),
                "--from" => asked.from = value.parse().unwrap_or(asked.from),
                "--to" => asked.to = value.parse().unwrap_or(asked.to),
                "--y" => asked.y = value.parse().unwrap_or(asked.y),
                "--step" => asked.step = value.parse().unwrap_or(asked.step),
                "--pause" => asked.pause = value.parse().unwrap_or(asked.pause),
                "--passes" => asked.passes = value.parse().unwrap_or(asked.passes),
                "--seconds" => asked.seconds = value.parse().unwrap_or(asked.seconds),
                "--width" => asked.width = value.parse().unwrap_or(asked.width),
                "--height" => asked.height = value.parse().unwrap_or(asked.height),
                "--log" => asked.log = value.clone(),
                _ => {}
            }
            at += 2;
        }
        asked
    }

    fn place(window: isize, x: i32, y: i32) {
        unsafe { SetWindowPos(window, 0, x, y, 0, 0, KEEP_SIZE | KEEP_ORDER | STAY_BACK) };
    }

    fn changes() -> usize {
        LINES.lock().map(|lines| lines.iter().filter(|line| line.contains("SCALE CHANGE")).count()).unwrap_or(0)
    }

    fn drive(mode: String, from: i32, to: i32, y: i32, step: i32, pause: u64, passes: u32) {
        let window = loop {
            let window = WINDOW.load(Ordering::Relaxed);
            if window != 0 {
                break window;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        std::thread::sleep(Duration::from_millis(500));
        place(window, from, y);
        std::thread::sleep(Duration::from_millis(900));
        let steps = ((to - from) / step.max(1)).max(1);
        for pass in 1..=passes {
            for (name, sign) in [("RIGHT", 1), ("LEFT", -1)] {
                let before = changes();
                note(format!("=== pass {pass} going {name} begins | {}", state(window)));
                for index in 1..=steps {
                    if mode == "rel" {
                        let rect = outer(window);
                        place(window, rect.left + sign * step, rect.top);
                    } else {
                        let x = if sign > 0 { from + index * step } else { to - index * step };
                        place(window, x, y);
                    }
                    std::thread::sleep(Duration::from_millis(pause));
                }
                std::thread::sleep(Duration::from_millis(900));
                note(format!(
                    "=== pass {pass} going {name} ends: {} scale change(s) in one crossing | {}",
                    changes() - before,
                    state(window)
                ));
            }
        }
        let _ = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        });
    }

    fn drive_by_mouse(from: i32, y: i32, grab: f64, travel: i32, step: i32, pause: u64, passes: u32) {
        let window = loop {
            let window = WINDOW.load(Ordering::Relaxed);
            if window != 0 {
                break window;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        std::thread::sleep(Duration::from_millis(500));
        unsafe { SetWindowPos(window, ABOVE_ALL, 0, 0, 0, 0, KEEP_SIZE | KEEP_PLACE | STAY_BACK) };
        place(window, from, y);
        std::thread::sleep(Duration::from_millis(900));
        let kept = pointer();
        'all: for pass in 1..=passes {
            for (name, sign) in [("RIGHT", 1), ("LEFT", -1)] {
                let rect = outer(window);
                let (_, high) = inner(window);
                let side = ((rect.right - rect.left) - inner(window).0) / 2;
                let bar = (rect.bottom - rect.top) - high - side;
                let hold = (rect.left + (f64::from(rect.right - rect.left) * grab).round() as i32, rect.top + (bar / 2).max(6));
                mouse(MOUSE_MOVES, hold.0, hold.1);
                std::thread::sleep(Duration::from_millis(120));
                let under = unsafe { GetAncestor(WindowFromPoint(Point { x: hold.0, y: hold.1 }), ROOT) };
                let spot = pointer();
                if under != window || (spot.x - hold.0).abs() > 3 || (spot.y - hold.1).abs() > 3 {
                    note(format!("the pointer is not on the test window's title bar (wanted {hold:?}, is ({},{})); nothing was pressed", spot.x, spot.y));
                    break 'all;
                }
                let before = changes();
                note(format!("=== pass {pass} dragged by the mouse going {name}, held {grab:.2} across the title bar, begins | {}", state(window)));
                mouse(LEFT_DOWN, hold.0, hold.1);
                std::thread::sleep(Duration::from_millis(150));
                let mut trail = String::new();
                let mut last = (0, 0u32);
                let mut flips = 0;
                for index in 1..=(travel / step.max(1)) {
                    mouse(MOUSE_MOVES, hold.0 + sign * index * step, hold.1);
                    std::thread::sleep(Duration::from_millis(pause));
                    let now = outer(window);
                    let seen = (now.right - now.left, unsafe { GetDpiForWindow(window) });
                    if seen != last {
                        if last.1 != 0 {
                            flips += 1;
                        }
                        trail.push_str(&format!(" [pointer {} left {} width {} dpi {}]", hold.0 + sign * index * step, now.left, seen.0, seen.1));
                        last = seen;
                    }
                }
                std::thread::sleep(Duration::from_millis(400));
                mouse(LEFT_UP, hold.0 + sign * travel, hold.1);
                std::thread::sleep(Duration::from_millis(900));
                note(format!("    as seen after each mouse step, when it changed:{trail}"));
                note(format!(
                    "=== pass {pass} going {name} ends: {} scale change(s), {flips} change(s) of width seen between steps | {}",
                    changes() - before,
                    state(window)
                ));
            }
        }
        mouse(MOUSE_MOVES, kept.x, kept.y);
        unsafe { SetCursorPos(kept.x, kept.y) };
        std::thread::sleep(Duration::from_millis(200));
        let _ = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        });
    }

    pub fn run() {
        let asked = asked();
        STARTED.get_or_init(Instant::now);
        let ui = Harness::new().expect("the window could not be made");
        ui.window().set_size(LogicalSize::new(asked.width, asked.height));
        let front = unsafe { GetForegroundWindow() };

        let watch = Rc::new(RefCell::new(ScaleWatch::default()));
        let hooked = Rc::new(Cell::new(false));
        let weak = ui.as_weak();
        let fix = asked.fix;
        let follow = asked.follow;
        let keep_front = asked.mode == "mouse";
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(33), move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            if !hooked.get() {
                let Some(window) = own_window(TITLE) else {
                    return;
                };
                let settled = fix && crate::platform::settle_scale_changes(TITLE);
                FIX.store(i64::from(settled), Ordering::Relaxed);
                note(format!("the fix is {}; the app's own following of the scale is {}", if settled { "ON (the app's own code)" } else { "off" }, if follow { "on" } else { "off" }));
                let previous = unsafe { GetWindowLongPtrW(window, WINDOW_PROCEDURE) };
                PREVIOUS.store(previous, Ordering::Relaxed);
                unsafe { SetWindowLongPtrW(window, WINDOW_PROCEDURE, watching as *const () as usize as isize) };
                hooked.set(true);
                if front != 0 && !keep_front {
                    unsafe { SetForegroundWindow(front) };
                }
                note(format!("window found | {}", state(window)));
                WINDOW.store(window, Ordering::Relaxed);
            }
            let window = ui.window();
            let scale = window.scale_factor();
            let size = window.size().to_logical(scale);
            if follow && watch.borrow_mut().changed(scale) {
                note(format!("    tick: the app sees scale {scale}, has the limits worked out again"));
                ui.set_scale_nudge(watch.borrow().nudge());
            }
            ui.set_note(format!("scale {scale}\n{:.0} x {:.0} logical", size.width, size.height).into());
        });

        if asked.mode == "mouse" {
            let (from, y, grab, travel, step, pause, passes) = (asked.from, asked.y, asked.grab, asked.travel, asked.step, asked.pause, asked.passes);
            let _ = std::thread::Builder::new()
                .name("drive".into())
                .spawn(move || drive_by_mouse(from, y, grab, travel, step, pause, passes));
        } else if asked.mode == "abs" || asked.mode == "rel" {
            let (mode, from, to, y, step, pause, passes) =
                (asked.mode.clone(), asked.from, asked.to, asked.y, asked.step, asked.pause, asked.passes);
            let _ = std::thread::Builder::new().name("drive".into()).spawn(move || drive(mode, from, to, y, step, pause, passes));
        } else {
            let seconds = asked.seconds;
            let _ = std::thread::Builder::new().name("wait".into()).spawn(move || {
                std::thread::sleep(Duration::from_secs(seconds));
                let _ = slint::invoke_from_event_loop(|| {
                    let _ = slint::quit_event_loop();
                });
            });
        }

        ui.run().expect("the window could not be shown");
        timer.stop();
        let lines = LINES.lock().map(|lines| lines.clone()).unwrap_or_default();
        let text = lines.join("\n");
        let _ = std::fs::write(&asked.log, &text);
        println!("{text}");
        println!("--- {} scale changes in all; log written to {}", changes(), asked.log);
    }
}
