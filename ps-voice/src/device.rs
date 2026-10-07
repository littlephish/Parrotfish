use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::Thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Data, ErrorKind, SampleFormat, StreamConfig};

use crate::capture::{level_db, SILENCE_DB};
use crate::codec::SAMPLE_RATE;
use crate::cues::{Cue, CueMixer};
use crate::playback::{soft_limit, BLOCK, MIX_CHANNELS};
use crate::resample::Resampler;
use crate::state::Shared;

const RETRY_DELAY: Duration = Duration::from_secs(2);
const REOPEN_DELAY: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
}

fn describe(device: &cpal::Device) -> Option<DeviceInfo> {
    let id = device.id().ok()?.to_string();
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_else(|_| id.clone());
    Some(DeviceInfo { id, name })
}

pub fn list_input_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    match host.input_devices() {
        Ok(devices) => devices.filter_map(|d| describe(&d)).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn list_output_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    match host.output_devices() {
        Ok(devices) => devices.filter_map(|d| describe(&d)).collect(),
        Err(_) => Vec::new(),
    }
}

pub(crate) enum Ctl {
    SetInput(Option<String>),
    SetOutput(Option<String>),
    InputFailed,
    OutputFailed,
    Stop,
}

pub(crate) struct FarSource {
    pub consumer: rtrb::Consumer<f32>,
    pub rate: u32,
}

pub(crate) struct InputSource {
    pub consumer: rtrb::Consumer<f32>,
    pub rate: u32,
}

fn find_device(host: &cpal::Host, id: &Option<String>, input: bool) -> Result<cpal::Device, String> {
    match id {
        Some(wanted) => {
            let devices = if input { host.input_devices() } else { host.output_devices() };
            let devices = devices.map_err(|e| format!("cannot list devices: {e}"))?;
            for device in devices {
                if device.id().map(|d| d.to_string() == *wanted).unwrap_or(false) {
                    return Ok(device);
                }
            }
            if input {
                for device in host.output_devices().map_err(|e| format!("cannot list devices: {e}"))? {
                    if device.id().map(|d| d.to_string() == *wanted).unwrap_or(false) {
                        return Ok(device);
                    }
                }
            }
            Err("the selected device is not available".into())
        }
        None => {
            let device = if input { host.default_input_device() } else { host.default_output_device() };
            device.ok_or_else(|| {
                if input { "no microphone found".to_string() } else { "no playback device found".to_string() }
            })
        }
    }
}

fn mono_from<T: Copy>(
    samples: &[T],
    channels: usize,
    convert: impl Fn(T) -> f32,
    producer: &mut rtrb::Producer<f32>,
) -> usize {
    let used = channels.clamp(1, 2);
    let mut dropped = 0;
    for frame in samples.chunks_exact(channels.max(1)) {
        let mut acc = 0.0;
        for sample in frame.iter().take(used) {
            acc += convert(*sample);
        }
        if producer.push(acc / used as f32).is_err() {
            dropped += 1;
        }
    }
    dropped
}

fn push_input(data: &Data, channels: usize, producer: &mut rtrb::Producer<f32>) -> Option<usize> {
    Some(match data.sample_format() {
        SampleFormat::F32 => mono_from(data.as_slice::<f32>()?, channels, |s| s, producer),
        SampleFormat::I16 => mono_from(data.as_slice::<i16>()?, channels, |s| s as f32 / 32768.0, producer),
        SampleFormat::U16 => {
            mono_from(data.as_slice::<u16>()?, channels, |s| (s as f32 - 32768.0) / 32768.0, producer)
        }
        SampleFormat::I32 => {
            mono_from(data.as_slice::<i32>()?, channels, |s| s as f32 / 2_147_483_648.0, producer)
        }
        SampleFormat::I8 => mono_from(data.as_slice::<i8>()?, channels, |s| s as f32 / 128.0, producer),
        SampleFormat::U8 => {
            mono_from(data.as_slice::<u8>()?, channels, |s| (s as f32 - 128.0) / 128.0, producer)
        }
        SampleFormat::F64 => mono_from(data.as_slice::<f64>()?, channels, |s| s as f32, producer),
        _ => return None,
    })
}

fn is_supported(format: SampleFormat) -> bool {
    matches!(
        format,
        SampleFormat::F32
            | SampleFormat::I16
            | SampleFormat::U16
            | SampleFormat::I32
            | SampleFormat::I8
            | SampleFormat::U8
            | SampleFormat::F64
    )
}

fn should_rebuild(kind: ErrorKind) -> bool {
    !matches!(kind, ErrorKind::Xrun | ErrorKind::RealtimeDenied | ErrorKind::DeviceChanged)
}

fn open_input(
    host: &cpal::Host,
    id: &Option<String>,
    shared: &Arc<Shared>,
    sources: &Sender<InputSource>,
    ctl: &Sender<Ctl>,
    tx_thread: &Thread,
) -> Result<(cpal::Stream, String), String> {
    let device = find_device(host, id, true)?;
    let supported = device
        .default_input_config()
        .or_else(|_| device.default_output_config())
        .map_err(|e| format!("no usable input format: {e}"))?;
    let format = supported.sample_format();
    if !is_supported(format) {
        return Err(format!("unsupported input sample format {format}"));
    }
    let config: StreamConfig = supported.config();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let (mut producer, consumer) = rtrb::RingBuffer::<f32>::new(rate as usize);
    sources
        .send(InputSource { consumer, rate })
        .map_err(|_| "audio transmit thread is gone".to_string())?;
    let cb_shared = shared.clone();
    let waker = tx_thread.clone();
    let err_ctl = ctl.clone();
    let stream = device
        .build_input_stream_raw(
            config,
            format,
            move |data: &Data, _| {
                match push_input(data, channels, &mut producer) {
                    Some(dropped) if dropped > 0 => {
                        cb_shared.capture_overruns.fetch_add(dropped as u64, Ordering::Relaxed);
                    }
                    _ => {}
                }
                cb_shared
                    .frames_captured
                    .fetch_add((data.len() / channels.max(1)) as u64, Ordering::Relaxed);
                waker.unpark();
            },
            move |err| {
                if should_rebuild(err.kind()) {
                    let _ = err_ctl.send(Ctl::InputFailed);
                }
            },
            None,
        )
        .map_err(|e| format!("cannot open microphone: {e}"))?;
    stream.play().map_err(|e| format!("cannot start microphone: {e}"))?;
    let name = describe(&device).map(|d| d.name).unwrap_or_else(|| "microphone".into());
    Ok((stream, format!("{name} ({rate} Hz, {channels} ch)")))
}

struct OutputRenderer {
    shared: Arc<Shared>,
    resampler: Resampler,
    fifo: VecDeque<f32>,
    block: Vec<f32>,
    converted: Vec<f32>,
    cues: CueMixer,
    waiting: Vec<Cue>,
    far: rtrb::Producer<f32>,
    tap: bool,
}

impl OutputRenderer {
    fn new(shared: Arc<Shared>, rate: u32, far: rtrb::Producer<f32>) -> Self {
        Self {
            far,
            tap: false,
            shared,
            resampler: Resampler::new(SAMPLE_RATE, rate, MIX_CHANNELS),
            fifo: VecDeque::new(),
            block: vec![0.0; BLOCK],
            converted: Vec::new(),
            cues: CueMixer::new(),
            waiting: Vec::new(),
        }
    }

    fn fill(&mut self, frames: usize) {
        self.tap = self.shared.echo_cancel();
        let needed = frames * MIX_CHANNELS;
        let mut guard = 0;
        while self.fifo.len() < needed && guard < 64 {
            guard += 1;
            let active = match self.shared.playback.lock() {
                Ok(mut playback) => playback.mix(&mut self.block),
                Err(_) => {
                    self.block.iter_mut().for_each(|s| *s = 0.0);
                    0
                }
            };
            let muted = self.shared.speaker_muted.load(Ordering::Relaxed);
            let volume = if muted { 0.0 } else { self.shared.output_volume() };
            if active > 0 {
                for s in self.block.iter_mut() {
                    *s = (*s * volume).clamp(-1.0, 1.0);
                }
            }
            let heard = self.shared.cue_volume() * self.shared.output_volume();
            self.shared.take_cues(&mut self.waiting);
            for cue in self.waiting.drain(..) {
                let silenced = muted && !matches!(cue, Cue::SoundOff | Cue::SoundOn);
                if heard > 0.0 && !silenced {
                    self.cues.start(cue);
                }
            }
            let cued = self.cues.mix(&mut self.block, MIX_CHANNELS, heard);
            if cued {
                for s in self.block.iter_mut() {
                    *s = soft_limit(*s);
                }
            }
            if active > 0 || cued {
                self.shared.set_output_level(level_db(&self.block));
            } else {
                self.shared.set_output_level(SILENCE_DB);
            }
            self.converted.clear();
            self.resampler.process(&self.block, &mut self.converted);
            self.fifo.extend(self.converted.iter().copied());
        }
        self.shared.frames_played.fetch_add(frames as u64, Ordering::Relaxed);
    }

    fn next_frame(&mut self) -> (f32, f32) {
        let left = self.fifo.pop_front().unwrap_or(0.0);
        let right = self.fifo.pop_front().unwrap_or(0.0);
        if self.tap {
            let _ = self.far.push((left + right) * 0.5);
        }
        (left, right)
    }
}

fn write_frames<T: Copy>(
    samples: &mut [T],
    channels: usize,
    renderer: &mut OutputRenderer,
    convert: impl Fn(f32) -> T,
) {
    let channels = channels.max(1);
    renderer.fill(samples.len() / channels);
    let silence = convert(0.0);
    for frame in samples.chunks_exact_mut(channels) {
        let (left, right) = renderer.next_frame();
        if channels == 1 {
            frame[0] = convert((left + right) * 0.5);
        } else {
            frame[0] = convert(left);
            frame[1] = convert(right);
            for extra in frame.iter_mut().skip(2) {
                *extra = silence;
            }
        }
    }
}

fn render_output(data: &mut Data, channels: usize, renderer: &mut OutputRenderer) {
    match data.sample_format() {
        SampleFormat::F32 => {
            if let Some(s) = data.as_slice_mut::<f32>() {
                write_frames(s, channels, renderer, |v| v);
            }
        }
        SampleFormat::I16 => {
            if let Some(s) = data.as_slice_mut::<i16>() {
                write_frames(s, channels, renderer, |v| (v * 32767.0) as i16);
            }
        }
        SampleFormat::U16 => {
            if let Some(s) = data.as_slice_mut::<u16>() {
                write_frames(s, channels, renderer, |v| (v * 32767.0 + 32768.0) as u16);
            }
        }
        SampleFormat::I32 => {
            if let Some(s) = data.as_slice_mut::<i32>() {
                write_frames(s, channels, renderer, |v| (v as f64 * 2_147_483_647.0) as i32);
            }
        }
        SampleFormat::I8 => {
            if let Some(s) = data.as_slice_mut::<i8>() {
                write_frames(s, channels, renderer, |v| (v * 127.0) as i8);
            }
        }
        SampleFormat::U8 => {
            if let Some(s) = data.as_slice_mut::<u8>() {
                write_frames(s, channels, renderer, |v| (v * 127.0 + 128.0) as u8);
            }
        }
        SampleFormat::F64 => {
            if let Some(s) = data.as_slice_mut::<f64>() {
                write_frames(s, channels, renderer, |v| v as f64);
            }
        }
        _ => {}
    }
}

fn open_output(
    host: &cpal::Host,
    id: &Option<String>,
    shared: &Arc<Shared>,
    ctl: &Sender<Ctl>,
    far_sources: &Sender<FarSource>,
) -> Result<(cpal::Stream, String), String> {
    let device = find_device(host, id, false)?;
    let supported = device.default_output_config().map_err(|e| format!("no usable output format: {e}"))?;
    let format = supported.sample_format();
    if !is_supported(format) {
        return Err(format!("unsupported output sample format {format}"));
    }
    let config: StreamConfig = supported.config();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let (far, consumer) = rtrb::RingBuffer::<f32>::new(rate as usize);
    far_sources
        .send(FarSource { consumer, rate })
        .map_err(|_| "audio transmit thread is gone".to_string())?;
    let mut renderer = OutputRenderer::new(shared.clone(), rate, far);
    let err_ctl = ctl.clone();
    let stream = device
        .build_output_stream_raw(
            config,
            format,
            move |data: &mut Data, _| render_output(data, channels, &mut renderer),
            move |err| {
                if should_rebuild(err.kind()) {
                    let _ = err_ctl.send(Ctl::OutputFailed);
                }
            },
            None,
        )
        .map_err(|e| format!("cannot open playback device: {e}"))?;
    stream.play().map_err(|e| format!("cannot start playback: {e}"))?;
    let name = describe(&device).map(|d| d.name).unwrap_or_else(|| "speakers".into());
    Ok((stream, format!("{name} ({rate} Hz, {channels} ch)")))
}

pub(crate) fn manage(
    shared: Arc<Shared>,
    rx: Receiver<Ctl>,
    ctl: Sender<Ctl>,
    sources: Sender<InputSource>,
    far_sources: Sender<FarSource>,
    tx_thread: Thread,
) {
    let host = cpal::default_host();
    let mut input_stream: Option<cpal::Stream> = None;
    let mut output_stream: Option<cpal::Stream> = None;
    let mut input_id: Option<String> = None;
    let mut output_id: Option<String> = None;
    let mut open_input_at = Some(Instant::now());
    let mut open_output_at = Some(Instant::now());

    loop {
        let now = Instant::now();
        if open_output_at.is_some_and(|t| now >= t) {
            output_stream = None;
            match open_output(&host, &output_id, &shared, &ctl, &far_sources) {
                Ok((stream, label)) => {
                    output_stream = Some(stream);
                    open_output_at = None;
                    if let Ok(mut status) = shared.status.lock() {
                        status.output = label;
                        status.output_ok = true;
                    }
                }
                Err(e) => {
                    open_output_at = Some(now + RETRY_DELAY);
                    if let Ok(mut status) = shared.status.lock() {
                        status.output = e;
                        status.output_ok = false;
                    }
                }
            }
        }
        if open_input_at.is_some_and(|t| now >= t) {
            input_stream = None;
            match open_input(&host, &input_id, &shared, &sources, &ctl, &tx_thread) {
                Ok((stream, label)) => {
                    input_stream = Some(stream);
                    open_input_at = None;
                    if let Ok(mut status) = shared.status.lock() {
                        status.input = label;
                        status.input_ok = true;
                    }
                }
                Err(e) => {
                    open_input_at = Some(now + RETRY_DELAY);
                    if let Ok(mut status) = shared.status.lock() {
                        status.input = e;
                        status.input_ok = false;
                    }
                }
            }
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ctl::SetInput(id)) => {
                input_id = id;
                open_input_at = Some(Instant::now());
            }
            Ok(Ctl::SetOutput(id)) => {
                output_id = id;
                open_output_at = Some(Instant::now());
            }
            Ok(Ctl::InputFailed) => {
                if open_input_at.is_none() {
                    input_stream = None;
                    open_input_at = Some(Instant::now() + REOPEN_DELAY);
                }
            }
            Ok(Ctl::OutputFailed) => {
                if open_output_at.is_none() {
                    output_stream = None;
                    open_output_at = Some(Instant::now() + REOPEN_DELAY);
                }
            }
            Ok(Ctl::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    drop(input_stream);
    drop(output_stream);
}
