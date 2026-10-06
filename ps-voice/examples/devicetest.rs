use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ps_voice::codec::{Encoder, CODEC_OPUS_VOICE, FRAME_SAMPLES, MAX_PACKET_BYTES, SAMPLE_RATE};
use ps_voice::AudioEngine;

const TONE_HZ: f32 = 1000.0;
const TEST_CLIENT: u16 = 4242;

fn goertzel(samples: &[f32], freq: f32, rate: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let w = 2.0 * std::f32::consts::PI * freq / rate;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0f32, 0f32);
    for x in samples {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    2.0 * power.max(0.0).sqrt() / samples.len() as f32
}

fn db(x: f32) -> f32 {
    if x <= 1e-9 {
        -180.0
    } else {
        20.0 * x.log10()
    }
}

struct Loopback {
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
}

fn open_loopback() -> Result<Loopback, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no default output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("loopback check needs f32, device uses {}", supported.sample_format()));
    }
    let config = supported.config();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let samples = Arc::new(Mutex::new(Vec::new()));
    let sink = samples.clone();
    let stream = device
        .build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if let Ok(mut buf) = sink.lock() {
                    for frame in data.chunks_exact(channels.max(1)) {
                        buf.push(frame[0]);
                    }
                }
            },
            |_| {},
            None,
        )
        .map_err(|e| format!("cannot open loopback capture: {e}"))?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Loopback { _stream: stream, samples, rate })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let play_tone = args.iter().any(|a| a == "--tone");
    let amplitude: f32 = args
        .iter()
        .position(|a| a == "--amplitude")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.004);

    println!("input devices:");
    for d in ps_voice::list_input_devices() {
        println!("  {}  [{}]", d.name, d.id);
    }
    println!("output devices:");
    for d in ps_voice::list_output_devices() {
        println!("  {}  [{}]", d.name, d.id);
    }

    let engine = AudioEngine::start();
    let shared = engine.shared().clone();
    if let Some(wanted) = args.iter().position(|a| a == "--input").and_then(|i| args.get(i + 1)) {
        match ps_voice::list_input_devices().into_iter().find(|d| d.name.contains(wanted.as_str())) {
            Some(device) => engine.set_input_device(Some(device.id)),
            None => println!("no input device matches \"{wanted}\", using the default"),
        }
    }
    std::thread::sleep(Duration::from_millis(1500));
    let status = engine.status();
    println!("microphone: {} (ok={})", status.input, status.input_ok);
    println!("speakers:   {} (ok={})", status.output, status.output_ok);

    let captured_0 = shared.frames_captured.load(Ordering::Relaxed);
    let played_0 = shared.frames_played.load(Ordering::Relaxed);
    let start = Instant::now();
    let mut min_level = f32::MAX;
    let mut max_level = f32::MIN;
    while start.elapsed() < Duration::from_secs(2) {
        let level = shared.input_level();
        min_level = min_level.min(level);
        max_level = max_level.max(level);
        std::thread::sleep(Duration::from_millis(20));
    }
    let seconds = start.elapsed().as_secs_f64();
    let captured = shared.frames_captured.load(Ordering::Relaxed) - captured_0;
    let played = shared.frames_played.load(Ordering::Relaxed) - played_0;
    println!(
        "capture:  {:.0} frames/s, input level {:.1} .. {:.1} dBFS, overruns {}",
        captured as f64 / seconds,
        min_level,
        max_level,
        shared.capture_overruns.load(Ordering::Relaxed)
    );
    println!("playback: {:.0} frames/s pulled by the device", played as f64 / seconds);
    let mut ok = status.input_ok && status.output_ok && captured > 0 && played > 0;
    if max_level <= -95.9 {
        println!("note: the microphone delivers digital silence (muted, retracted, or blocked by Windows privacy settings)");
    }

    if play_tone {
        println!(
            "playing a {TONE_HZ} Hz test tone at {:.0} dBFS for 1.5 s through the normal decode/mix path",
            db(amplitude)
        );
        match open_loopback() {
            Ok(loopback) => {
                std::thread::sleep(Duration::from_millis(700));
                let baseline: Vec<f32> = loopback.samples.lock().map(|mut b| std::mem::take(&mut *b)).unwrap_or_default();
                let mut encoder = Encoder::new(CODEC_OPUS_VOICE, 10).expect("encoder");
                let mut packet = [0u8; MAX_PACKET_BYTES];
                let tone_start = Instant::now();
                for f in 0..75usize {
                    let pcm: Vec<f32> = (0..FRAME_SAMPLES)
                        .map(|i| {
                            let t = (f * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32;
                            amplitude * (2.0 * std::f32::consts::PI * TONE_HZ * t).sin()
                        })
                        .collect();
                    let n = encoder.encode(&pcm, &mut packet).expect("encode");
                    engine.push_voice(0, TEST_CLIENT, f as u16, CODEC_OPUS_VOICE, &packet[..n]);
                    let due = tone_start + Duration::from_millis(20 * (f as u64 + 1));
                    if let Some(wait) = due.checked_duration_since(Instant::now()) {
                        std::thread::sleep(wait);
                    }
                }
                engine.push_voice(0, TEST_CLIENT, 75, CODEC_OPUS_VOICE, &[]);
                std::thread::sleep(Duration::from_millis(400));
                let during: Vec<f32> = loopback.samples.lock().map(|mut b| std::mem::take(&mut *b)).unwrap_or_default();
                let rate = loopback.rate as f32;
                let skip = (rate * 0.4) as usize;
                let take = (rate * 0.9) as usize;
                let body: &[f32] = if during.len() > skip + take { &during[skip..skip + take] } else { &during[..] };
                let tone = goertzel(body, TONE_HZ, rate);
                let before = goertzel(&baseline, TONE_HZ, rate);
                println!(
                    "loopback capture of the output device: {} Hz, tone {:.1} dBFS while playing (expected {:.1}), {:.1} dBFS before",
                    loopback.rate,
                    db(tone),
                    db(amplitude),
                    db(before)
                );
                let level_ok = (db(tone) - db(amplitude)).abs() < 3.0;
                let rise_ok = db(tone) - db(before) > 10.0;
                if level_ok && rise_ok {
                    println!("sound output verified: the tone reached the output device at the expected level");
                } else {
                    println!("sound output NOT verified (level_ok={level_ok}, rise_ok={rise_ok})");
                    ok = false;
                }
            }
            Err(e) => {
                println!("loopback check unavailable: {e}");
            }
        }
    }

    drop(engine);
    println!("{}", if ok { "PASS" } else { "FAIL" });
    std::process::exit(if ok { 0 } else { 1 });
}
