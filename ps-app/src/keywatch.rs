use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::hotkeys::{evaluate, usable, Bindings, Capture, CaptureStep, Edges, Held, Latch};
use crate::platform;

#[derive(Default)]
pub struct WatchState {
    bindings: Mutex<Bindings>,
    delay_ms: AtomicU32,
    capturing: AtomicBool,
    captured: Mutex<Option<CaptureStep>>,
    fired: AtomicU32,
    stop: AtomicBool,
}

#[derive(Default)]
pub struct WatchCore {
    latch: Latch,
    capture: Option<Capture>,
    edges: Edges,
}

impl WatchCore {
    pub fn step(&mut self, state: &WatchState, now: Instant, down: &dyn Fn(u16) -> bool) -> Held {
        if state.capturing.load(Ordering::Relaxed) {
            let keys: Vec<u16> = (3u16..=254).filter(|vk| usable(*vk) && down(*vk)).collect();
            let step = self.capture.get_or_insert_with(Capture::new).feed(&keys);
            if step != CaptureStep::Waiting {
                if let Ok(mut slot) = state.captured.lock() {
                    *slot = Some(step);
                }
                state.capturing.store(false, Ordering::Relaxed);
                self.capture = None;
            }
            self.latch = Latch::default();
            self.edges.block();
            return Held::default();
        }
        self.capture = None;
        let raw = match state.bindings.lock() {
            Ok(bindings) => {
                let fired = self.edges.update(&bindings.actions, down);
                if fired != 0 {
                    state.fired.fetch_or(fired, Ordering::Relaxed);
                }
                evaluate(&bindings, down)
            }
            Err(_) => Held::default(),
        };
        self.latch.update(now, raw, Duration::from_millis(u64::from(state.delay_ms.load(Ordering::Relaxed))))
    }
}

impl WatchState {
    pub fn set_bindings(&self, bindings: Bindings) {
        if let Ok(mut slot) = self.bindings.lock() {
            *slot = bindings;
        }
    }

    pub fn set_release_delay(&self, ms: u32) {
        self.delay_ms.store(ms.min(1000), Ordering::Relaxed);
    }

    pub fn begin_capture(&self) {
        if let Ok(mut slot) = self.captured.lock() {
            *slot = None;
        }
        self.capturing.store(true, Ordering::Relaxed);
    }

    pub fn cancel_capture(&self) {
        self.capturing.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.captured.lock() {
            *slot = None;
        }
    }

    pub fn take_fired(&self) -> u32 {
        self.fired.swap(0, Ordering::Relaxed)
    }

    pub fn take_captured(&self) -> Option<CaptureStep> {
        self.captured.lock().ok().and_then(|mut slot| slot.take())
    }
}

pub struct KeyWatcher {
    state: Arc<WatchState>,
    thread: Option<JoinHandle<()>>,
}

impl KeyWatcher {
    pub fn start(audio: Arc<ps_voice::Shared>) -> Self {
        let state = Arc::new(WatchState::default());
        let shared = state.clone();
        let thread = std::thread::Builder::new()
            .name("ps-keys".into())
            .spawn(move || {
                let mut core = WatchCore::default();
                while !shared.stop.load(Ordering::Relaxed) {
                    let held = core.step(&shared, Instant::now(), &|vk| platform::key_down(i32::from(vk)));
                    audio.set_keys(held.talk, held.lane);
                    std::thread::sleep(Duration::from_millis(5));
                }
                audio.set_keys(false, 0);
            })
            .expect("failed to spawn the key watcher thread");
        Self { state, thread: Some(thread) }
    }

    pub fn state(&self) -> &WatchState {
        &self.state
    }
}

impl Drop for KeyWatcher {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::{Bindings, CaptureStep, Chord, Held};

    #[test]
    fn the_watcher_follows_the_keys_and_sends_nothing_while_choosing_one() {
        let state = WatchState::default();
        state.set_bindings(Bindings {
            talk: vec![Chord::new(&[0x87])],
            whisper: vec![Chord::new(&[0x86])],
            reply: Chord::default(),
            actions: vec![Chord::new(&[0x88])],
        });
        let mut core = WatchCore::default();
        let now = Instant::now();
        assert_eq!(core.step(&state, now, &|vk| vk == 0x87), Held { talk: true, lane: 0 });
        assert_eq!(core.step(&state, now, &|vk| vk == 0x86), Held { talk: false, lane: 1 });
        state.begin_capture();
        assert_eq!(core.step(&state, now, &|vk| vk == 0x87), Held::default());
        assert_eq!(core.step(&state, now, &|_| false), Held::default());
        assert_eq!(core.step(&state, now, &|vk| vk == 0x41), Held::default());
        assert_eq!(state.take_captured(), None);
        assert_eq!(core.step(&state, now, &|_| false), Held::default());
        assert_eq!(state.take_captured(), Some(CaptureStep::Done(Chord::new(&[0x41]))));
        assert_eq!(state.take_captured(), None);
        assert_eq!(core.step(&state, now, &|vk| vk == 0x87), Held { talk: true, lane: 0 });

        state.begin_capture();
        assert_eq!(core.step(&state, now, &|_| false), Held::default());
        state.cancel_capture();
        assert_eq!(state.take_captured(), None);
        assert_eq!(core.step(&state, now, &|vk| vk == 0x87), Held { talk: true, lane: 0 });

        assert_eq!(state.take_fired(), 0);
        core.step(&state, now, &|_| false);
        core.step(&state, now, &|vk| vk == 0x88);
        core.step(&state, now, &|vk| vk == 0x88);
        assert_eq!(state.take_fired(), 1, "one press of an action key is reported once");
        assert_eq!(state.take_fired(), 0);
        state.begin_capture();
        core.step(&state, now, &|_| false);
        core.step(&state, now, &|vk| vk == 0x88);
        core.step(&state, now, &|_| false);
        assert_eq!(state.take_captured(), Some(CaptureStep::Done(Chord::new(&[0x88]))));
        assert_eq!(state.take_fired(), 0, "choosing a key is not using it");
        core.step(&state, now, &|_| false);
        core.step(&state, now, &|vk| vk == 0x88);
        assert_eq!(state.take_fired(), 1);
    }
}
