pub mod agc;
pub mod capture;
pub mod codec;
pub mod cues;
pub mod denoise;
mod device;
pub mod echo;
pub mod level;
pub mod playback;
pub mod resample;
pub mod state;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub use cues::{cue_samples, Cue};
pub use device::{list_input_devices, list_output_devices, DeviceInfo};
pub use playback::Adjustment;
pub use state::{DeviceStatus, FrameSink, Shared, TxMode, LANES, LOOPBACK_CLIENT_ID, LOOPBACK_SESSION};

use capture::Transmitter;
use device::{Ctl, FarSource, InputSource};

pub struct AudioEngine {
    shared: Arc<Shared>,
    ctl: Sender<Ctl>,
    stop: Arc<AtomicBool>,
    tx_thread: Option<JoinHandle<()>>,
    manager_thread: Option<JoinHandle<()>>,
}

fn transmit_loop(
    shared: Arc<Shared>,
    sources: Receiver<InputSource>,
    far_sources: Receiver<FarSource>,
    stop: Arc<AtomicBool>,
) {
    let mut source: Option<InputSource> = None;
    let mut transmitter = Transmitter::new(codec::SAMPLE_RATE);
    let mut chunk: Vec<f32> = Vec::with_capacity(4800);
    let mut loopback_seq: u16 = 0;
    while !stop.load(Ordering::Relaxed) {
        while let Ok(next) = sources.try_recv() {
            transmitter.set_input_rate(next.rate);
            transmitter.restart_echo();
            source = Some(next);
        }
        while let Ok(next) = far_sources.try_recv() {
            transmitter.set_far(next.consumer, next.rate);
        }
        chunk.clear();
        if let Some(src) = source.as_mut() {
            while chunk.len() < 9600 {
                match src.consumer.pop() {
                    Ok(sample) => chunk.push(sample),
                    Err(_) => break,
                }
            }
        }
        if chunk.is_empty() {
            std::thread::park_timeout(Duration::from_millis(10));
            continue;
        }
        let loopback = shared.loopback.load(Ordering::Relaxed);
        let mut sink_guard = shared.sink.lock().ok();
        let has_sink = sink_guard.as_ref().is_some_and(|g| g.is_some());
        shared.tx_enabled.store(has_sink || loopback, Ordering::Relaxed);
        let shared_ref = &*shared;
        transmitter.process(&chunk, shared_ref, &mut |lane, codec, data| {
            if let Some(guard) = sink_guard.as_mut() {
                if let Some(sink) = guard.as_mut() {
                    sink(lane, codec, data);
                }
            }
            if loopback {
                if let Ok(mut playback) = shared_ref.playback.lock() {
                    playback.push(LOOPBACK_SESSION, LOOPBACK_CLIENT_ID, loopback_seq, codec, data);
                }
                loopback_seq = loopback_seq.wrapping_add(1);
            }
        });
    }
}

impl AudioEngine {
    pub fn start() -> Self {
        let shared = Arc::new(Shared::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (ctl, ctl_rx) = mpsc::channel();
        let (source_tx, source_rx) = mpsc::channel();
        let (far_tx, far_rx) = mpsc::channel();

        let tx_shared = shared.clone();
        let tx_stop = stop.clone();
        let tx_thread = std::thread::Builder::new()
            .name("ps-voice-tx".into())
            .spawn(move || transmit_loop(tx_shared, source_rx, far_rx, tx_stop))
            .expect("failed to spawn audio transmit thread");

        let manager_shared = shared.clone();
        let manager_ctl = ctl.clone();
        let waker = tx_thread.thread().clone();
        let manager_thread = std::thread::Builder::new()
            .name("ps-voice-devices".into())
            .spawn(move || device::manage(manager_shared, ctl_rx, manager_ctl, source_tx, far_tx, waker))
            .expect("failed to spawn audio device thread");

        Self {
            shared,
            ctl,
            stop,
            tx_thread: Some(tx_thread),
            manager_thread: Some(manager_thread),
        }
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub fn set_input_device(&self, id: Option<String>) {
        let _ = self.ctl.send(Ctl::SetInput(id));
    }

    pub fn set_output_device(&self, id: Option<String>) {
        let _ = self.ctl.send(Ctl::SetOutput(id));
    }

    pub fn set_frame_sink(&self, sink: Option<FrameSink>) {
        if let Ok(mut slot) = self.shared.sink.lock() {
            *slot = sink;
        }
    }

    pub fn pause_transmit<R>(&self, f: impl FnOnce() -> R) -> R {
        let _held = self.shared.sink.lock();
        f()
    }

    pub fn push_voice(&self, session: u16, client_id: u16, voice_id: u16, codec: u8, data: &[u8]) {
        if let Ok(mut playback) = self.shared.playback.lock() {
            playback.push(session, client_id, voice_id, codec, data);
        }
    }

    pub fn set_volume(&self, session: u16, client_id: u16, volume: f32) {
        if let Ok(mut playback) = self.shared.playback.lock() {
            playback.set_volume(session, client_id, volume);
        }
    }

    pub fn set_priority(&self, session: u16, client_id: u16, on: bool) {
        if let Ok(mut playback) = self.shared.playback.lock() {
            playback.set_priority(session, client_id, on);
        }
    }

    pub fn set_priority_dim(&self, session: u16, db: Option<f32>) {
        if let Ok(mut playback) = self.shared.playback.lock() {
            playback.set_priority_dim(session, db);
        }
    }

    pub fn set_leveling(&self, on: bool) {
        if let Ok(mut playback) = self.shared.playback.lock() {
            playback.set_leveling(on);
        }
    }

    pub fn adjustment(&self, session: u16, client_id: u16) -> Adjustment {
        self.shared.playback.lock().map(|playback| playback.adjustment(session, client_id)).unwrap_or_default()
    }

    pub fn remove_talker(&self, session: u16, client_id: u16) {
        if let Ok(mut playback) = self.shared.playback.lock() {
            playback.remove(session, client_id);
        }
    }

    pub fn clear_session(&self, session: u16) {
        if let Ok(mut playback) = self.shared.playback.lock() {
            playback.clear_session(session);
        }
    }

    pub fn clear_talkers(&self) {
        if let Ok(mut playback) = self.shared.playback.lock() {
            playback.clear();
        }
    }

    pub fn take_unsupported_codec(&self) -> Option<u8> {
        self.shared.playback.lock().ok().and_then(|mut p| p.take_unsupported_codec())
    }

    pub fn set_codec(&self, codec: u8, quality: u8) {
        self.shared.codec.store(codec, Ordering::Relaxed);
        self.shared.codec_quality.store(quality, Ordering::Relaxed);
    }

    pub fn set_mic_muted(&self, muted: bool) {
        self.shared.mic_muted.store(muted, Ordering::Relaxed);
    }

    pub fn set_speaker_muted(&self, muted: bool) {
        self.shared.speaker_muted.store(muted, Ordering::Relaxed);
    }

    pub fn play_cue(&self, cue: Cue) {
        self.shared.queue_cue(cue);
    }

    pub fn set_loopback(&self, enabled: bool) {
        self.shared.loopback.store(enabled, Ordering::Relaxed);
        if !enabled {
            self.remove_talker(LOOPBACK_SESSION, LOOPBACK_CLIENT_ID);
        }
    }

    pub fn is_transmitting(&self) -> bool {
        self.shared.transmitting.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> DeviceStatus {
        self.shared.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.ctl.send(Ctl::Stop);
        if let Some(handle) = self.tx_thread.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
        if let Some(handle) = self.manager_thread.take() {
            let _ = handle.join();
        }
    }
}
