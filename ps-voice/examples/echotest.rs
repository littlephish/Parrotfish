use std::time::{Duration, Instant};

use ps_voice::codec::{Encoder, CODEC_OPUS_VOICE, FRAME_SAMPLES, MAX_PACKET_BYTES};
use ps_voice::{list_input_devices, list_output_devices, AudioEngine, DeviceInfo};

const FRAME: Duration = Duration::from_millis(20);

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn pick(devices: &[DeviceInfo], wanted: &str) -> Option<DeviceInfo> {
    let wanted = wanted.to_lowercase();
    devices.iter().find(|d| d.name.to_lowercase().contains(&wanted)).cloned()
}

struct Speech {
    seed: u64,
    low: f32,
    mid: f32,
    loudness: f32,
    target: f32,
    hold: usize,
}

impl Speech {
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        ((self.seed >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }

    fn frame(&mut self, level: f32) -> Vec<f32> {
        (0..FRAME_SAMPLES)
            .map(|_| {
                if self.hold == 0 {
                    self.target = if self.noise() > -0.5 { 0.4 + 0.6 * self.noise().abs() } else { 0.0 };
                    self.hold = 2400 + (self.noise().abs() * 9600.0) as usize;
                }
                self.hold -= 1;
                self.loudness += (self.target - self.loudness) * 0.002;
                let white = self.noise();
                self.low += 0.12 * (white - self.low);
                self.mid += 0.5 * (white - self.mid);
                level * self.loudness * (2.2 * self.low + 0.35 * self.mid)
            })
            .collect()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        eprintln!(
            "usage: echotest [--output NAME] [--input NAME | --loopback] [--seconds N] [--level DB]\nPlays a speech-like test sound through the output device and measures how loudly the input device\nhears it, first with echo cancelling off and then with it on. The sound is audible unless the output\nis a virtual device. With --loopback the output device's own signal is used as the microphone,\nwhich checks the canceller on real device timing without a loudspeaker or a room."
        );
        std::process::exit(2);
    }
    let seconds: f32 = arg_value(&args, "--seconds").and_then(|v| v.parse().ok()).unwrap_or(8.0);
    let level_db: f32 = arg_value(&args, "--level").and_then(|v| v.parse().ok()).unwrap_or(-30.0);
    let level = 10f32.powf(level_db.clamp(-80.0, -6.0) / 20.0) * 2.5;

    let engine = AudioEngine::start();
    let loopback = args.iter().any(|a| a == "--loopback");
    if let Some(wanted) = arg_value(&args, "--output") {
        match pick(&list_output_devices(), &wanted) {
            Some(device) => {
                if loopback {
                    engine.set_input_device(Some(device.id.clone()));
                }
                engine.set_output_device(Some(device.id));
            }
            None => {
                eprintln!("no output device has \"{wanted}\" in its name");
                std::process::exit(2);
            }
        }
    }
    if let Some(wanted) = arg_value(&args, "--input") {
        match pick(&list_input_devices(), &wanted) {
            Some(device) => engine.set_input_device(Some(device.id)),
            None => {
                eprintln!("no input device has \"{wanted}\" in its name");
                std::process::exit(2);
            }
        }
    }
    std::thread::sleep(Duration::from_millis(1500));
    let status = engine.status();
    println!("playing through: {}", status.output);
    println!("listening on:    {}", status.input);
    if !status.input_ok || !status.output_ok {
        eprintln!("a device could not be opened");
        std::process::exit(1);
    }

    let mut encoder = match Encoder::new(CODEC_OPUS_VOICE, 10) {
        Ok(encoder) => encoder,
        Err(e) => {
            eprintln!("cannot create the encoder: {e}");
            std::process::exit(1);
        }
    };
    let mut speech = Speech { seed: 0x9E37_79B9_7F4A_7C15, low: 0.0, mid: 0.0, loudness: 0.0, target: 0.0, hold: 0 };
    let mut packet = [0u8; MAX_PACKET_BYTES];
    let mut sequence: u16 = 0;
    let frames = (seconds * 50.0) as usize;
    let mut heard = [0.0f32; 2];
    for (round, on) in [false, true].into_iter().enumerate() {
        engine.shared().set_echo_cancel(on);
        let mut next = Instant::now();
        let (mut sum, mut count) = (0.0f64, 0u32);
        for index in 0..frames {
            let pcm = speech.frame(level);
            if let Ok(n) = encoder.encode(&pcm, &mut packet) {
                engine.push_voice(1, 1, sequence, CODEC_OPUS_VOICE, &packet[..n]);
                sequence = sequence.wrapping_add(1);
            }
            next += FRAME;
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
            if index >= frames / 2 {
                sum += 10f64.powf(f64::from(engine.shared().input_level()) / 10.0);
                count += 1;
            }
        }
        heard[round] = (10.0 * (sum / f64::from(count.max(1))).log10()) as f32;
        let shown = engine.shared().echo_reduction();
        println!(
            "echo cancelling {}: the microphone hears {:.1} dBFS{}",
            if on { "on " } else { "off" },
            heard[round],
            shown.map(|db| format!(" (the canceller reports {db:.1} dB removed)")).unwrap_or_default()
        );
    }
    engine.push_voice(1, 1, sequence, CODEC_OPUS_VOICE, &[]);
    match engine.shared().echo_delay_ms() {
        Some(ms) => println!("the sound came back about {ms:.0} ms after it was played"),
        None => println!("the canceller could not tell when the sound came back"),
    }
    println!("clock difference between the two devices: {:.0} parts per million", engine.shared().echo_drift_ppm());
    println!("difference: {:.1} dB", heard[0] - heard[1]);
    if heard[0] < -80.0 {
        println!("note: the input device heard next to nothing even with cancelling off, so there was no echo to remove");
    }
}
