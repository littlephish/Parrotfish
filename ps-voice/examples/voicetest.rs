use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ps_client::{ClientHandle, ConnectOptions, Event, TextTarget, VoiceSink};
use ps_identity::Identity;
use ps_voice::codec::{Encoder, CODEC_OPUS_MUSIC, CODEC_OPUS_VOICE, FRAME_SAMPLES, MAX_PACKET_BYTES, SAMPLE_RATE};
use ps_voice::playback::{Playback, BLOCK};

const TONE_HZ: f32 = 660.0;
const AMPLITUDE: f32 = 0.4;

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn goertzel(samples: &[f32], freq: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * freq / SAMPLE_RATE as f32;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0f32, 0f32);
    for x in samples {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    2.0 * power.max(0.0).sqrt() / samples.len().max(1) as f32
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

struct Peer {
    handle: ClientHandle,
    events: Receiver<Event>,
    name: String,
    client_id: u16,
    chat: Vec<String>,
    talking_on: u32,
    talking_off: u32,
    errors: Vec<String>,
    log: bool,
}

impl Peer {
    fn pump(&mut self) -> Result<(), String> {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Connected { client_id, server } => {
                    self.client_id = client_id;
                    println!(
                        "  {} connected as client {} to \"{}\" ({}), voice encryption mode {}",
                        self.name, client_id, server.name, server.version, server.codec_encryption_mode
                    );
                }
                Event::TextMessage { text, target, .. } => {
                    if target == TextTarget::Channel {
                        self.chat.push(text);
                    }
                }
                Event::Talking { talking, .. } => {
                    if talking {
                        self.talking_on += 1;
                    } else {
                        self.talking_off += 1;
                    }
                }
                Event::ServerError { id, message, extra } => {
                    self.errors.push(format!("{id:#06x} {message} {extra}"))
                }
                Event::Log(text) => {
                    if self.log {
                        println!("  [{}] {}", self.name, text);
                    }
                }
                Event::Disconnected { reason } => return Err(format!("{} disconnected: {reason}", self.name)),
                _ => {}
            }
        }
        Ok(())
    }
}

fn connect(host: &str, port: u16, name: &str, sink: Option<VoiceSink>, log: bool) -> Peer {
    let identity = Identity::generate(name, name);
    let mut options = ConnectOptions::new(host, port, identity);
    options.nickname = name.to_string();
    options.log_commands = log;
    let args: Vec<String> = std::env::args().collect();
    options.simulated_loss = arg_value(&args, "--loss").and_then(|p| p.parse().ok()).unwrap_or(0.0);
    let (tx, rx) = mpsc::channel();
    Peer {
        handle: ClientHandle::connect(options, tx, sink),
        events: rx,
        name: name.to_string(),
        client_id: 0,
        chat: Vec::new(),
        talking_on: 0,
        talking_off: 0,
        errors: Vec::new(),
        log,
    }
}

fn wait_until(peers: &mut [&mut Peer], timeout: Duration, mut done: impl FnMut(&[&mut Peer]) -> bool) -> Result<(), String> {
    let start = Instant::now();
    loop {
        for p in peers.iter_mut() {
            p.pump()?;
        }
        if done(peers) {
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err("timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return Err("usage: voicetest <host> [--port N] [--seconds N] [--music] [--log]".into());
    }
    let host = args[0].clone();
    let port = arg_value(&args, "--port").and_then(|p| p.parse().ok()).unwrap_or(9987);
    let seconds: usize = arg_value(&args, "--seconds").and_then(|p| p.parse().ok()).unwrap_or(4);
    let log = args.iter().any(|a| a == "--log");
    let codec = if args.iter().any(|a| a == "--music") { CODEC_OPUS_MUSIC } else { CODEC_OPUS_VOICE };

    let playback = Arc::new(Mutex::new(Playback::new()));
    let received = Arc::new(AtomicU64::new(0));
    let received_bytes = Arc::new(AtomicU64::new(0));
    let sink: VoiceSink = {
        let playback = playback.clone();
        let received = received.clone();
        let received_bytes = received_bytes.clone();
        Box::new(move |p| {
            if !p.data.is_empty() {
                received.fetch_add(1, Ordering::Relaxed);
                received_bytes.fetch_add(p.data.len() as u64, Ordering::Relaxed);
            }
            if let Ok(mut pb) = playback.lock() {
                pb.push(0, p.client_id, p.voice_id, p.codec, p.data);
            }
        })
    };

    if args.iter().any(|a| a == "--listen") {
        return listen(&host, port, seconds, log);
    }
    if let Some(count) = arg_value(&args, "--burst").and_then(|p| p.parse::<usize>().ok()) {
        return burst(&host, port, count, log);
    }

    println!("connecting two clients to {host}:{port}");
    let mut sender = connect(&host, port, "PhishSender", None, log);
    let mut listener = connect(&host, port, "PhishListener", Some(sink), log);
    let connected = wait_until(&mut [&mut sender, &mut listener], Duration::from_secs(15), |p| {
        p.iter().all(|x| x.handle.is_connected() && x.handle.own_channel() != 0)
    });
    connected.map_err(|e| format!("connect: {e}"))?;
    if sender.handle.own_channel() != listener.handle.own_channel() {
        return Err("clients ended up in different channels".into());
    }
    std::thread::sleep(Duration::from_millis(500));

    println!("chat: short, long compressible and long incompressible messages");
    let short = "hello / from | PhishSender \\ with\ttabs and ünïcödé ☺".to_string();
    let compressible = "the quick brown fox jumps over the lazy dog ".repeat(22).trim_end().to_string();
    let mut state = 0x2545_f491u32;
    let incompressible: String = (0..1000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[(state % 62) as usize] as char
        })
        .collect();
    for text in [&short, &compressible, &incompressible] {
        sender.handle.send_text(TextTarget::Channel, text);
    }
    let delivered =
        wait_until(&mut [&mut sender, &mut listener], Duration::from_secs(8), |p| p[1].chat.len() >= 3);
    if let Err(e) = delivered {
        return Err(format!(
            "chat delivery: {e} (listener got {} of 3; errors {:?})",
            listener.chat.len(),
            sender.errors
        ));
    }
    if listener.chat[0] != short || listener.chat[1] != compressible || listener.chat[2] != incompressible {
        return Err("chat text was altered in transit".into());
    }
    println!(
        "  ok: {} / {} / {} characters delivered intact (fragmentation + compression)",
        short.chars().count(),
        compressible.len(),
        incompressible.len()
    );

    let frames = seconds * 50;
    println!("voice: sending {frames} Opus frames (codec {codec}, {TONE_HZ} Hz tone)");
    let tx_handle = sender.handle.clone();
    let sent_bytes = Arc::new(AtomicU64::new(0));
    let sent_bytes_tx = sent_bytes.clone();
    let tx_thread = std::thread::spawn(move || -> Result<(), String> {
        let mut encoder = Encoder::new(codec, 6)?;
        let channels = encoder.channels();
        let mut packet = [0u8; MAX_PACKET_BYTES];
        let start = Instant::now();
        for f in 0..frames {
            let mut pcm = Vec::with_capacity(FRAME_SAMPLES * channels);
            for i in 0..FRAME_SAMPLES {
                let t = (f * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32;
                let s = AMPLITUDE * (2.0 * std::f32::consts::PI * TONE_HZ * t).sin();
                for _ in 0..channels {
                    pcm.push(s);
                }
            }
            let n = encoder.encode(&pcm, &mut packet)?;
            sent_bytes_tx.fetch_add(n as u64, Ordering::Relaxed);
            tx_handle.send_voice(codec, &packet[..n]);
            let due = start + Duration::from_millis(20 * (f as u64 + 1));
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        tx_handle.send_voice(codec, &[]);
        Ok(())
    });

    let mut pcm: Vec<f32> = Vec::new();
    let mut block = vec![0f32; BLOCK];
    let start = Instant::now();
    let mut pulls = 0u64;
    let total_time = Duration::from_millis(20 * frames as u64 + 1500);
    while start.elapsed() < total_time {
        sender.pump()?;
        listener.pump()?;
        let due_pulls = start.elapsed().as_millis() as u64 / 20;
        while pulls < due_pulls {
            pulls += 1;
            let active = playback.lock().map(|mut p| p.mix(&mut block)).unwrap_or(0);
            if active > 0 {
                pcm.extend_from_slice(&block);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    tx_thread.join().map_err(|_| "sender thread panicked".to_string())??;
    sender.pump()?;
    listener.pump()?;

    let got = received.load(Ordering::Relaxed);
    let stats = playback
        .lock()
        .ok()
        .and_then(|p| p.talker(0, sender.client_id).map(|t| t.stats))
        .ok_or("the listener never created a decoder for the sender")?;
    let left: Vec<f32> = pcm.chunks(2).map(|c| c[0]).collect();
    let body = if left.len() > 48_000 { &left[24_000..left.len() - 12_000] } else { &left[..] };
    let level = rms(body);
    let tone = goertzel(body, TONE_HZ);
    let off_tone = goertzel(body, TONE_HZ * 1.7);
    println!(
        "  received {got}/{frames} packets ({} B sent, {} B received), decoded {}, concealed {}, lost {}, late {}, underruns {}",
        sent_bytes.load(Ordering::Relaxed),
        received_bytes.load(Ordering::Relaxed),
        stats.decoded,
        stats.concealed,
        stats.lost,
        stats.late,
        stats.underruns
    );
    println!(
        "  decoded audio: {:.2} s, rms {:.3} (expected {:.3}), {TONE_HZ} Hz amplitude {:.3} (expected {:.2}), off-tone {:.4}",
        left.len() as f32 / SAMPLE_RATE as f32,
        level,
        AMPLITUDE / 2f32.sqrt(),
        tone,
        AMPLITUDE,
        off_tone
    );
    println!(
        "  talk indicator events at listener: {} start, {} stop",
        listener.talking_on, listener.talking_off
    );

    sender.handle.disconnect("voicetest done");
    listener.handle.disconnect("voicetest done");
    std::thread::sleep(Duration::from_millis(400));

    let mut failures = Vec::new();
    if (got as usize) < frames * 95 / 100 {
        failures.push(format!("only {got} of {frames} voice packets arrived"));
    }
    if received_bytes.load(Ordering::Relaxed) != sent_bytes.load(Ordering::Relaxed) && got as usize == frames {
        failures.push("voice payload size changed in transit".to_string());
    }
    if left.len() < frames * FRAME_SAMPLES * 9 / 10 {
        failures.push(format!("only {} samples were decoded", left.len()));
    }
    if (tone - AMPLITUDE).abs() > 0.08 {
        failures.push(format!("tone amplitude {tone:.3} is off"));
    }
    if off_tone > 0.02 {
        failures.push(format!("unexpected energy off the tone ({off_tone:.4})"));
    }
    if listener.talking_on == 0 || listener.talking_off == 0 {
        failures.push("talk indicator events are missing".to_string());
    }
    if !sender.errors.is_empty() || !listener.errors.is_empty() {
        failures.push(format!("server errors: {:?} {:?}", sender.errors, listener.errors));
    }
    if failures.is_empty() {
        println!("PASS");
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn burst(host: &str, port: u16, count: usize, log: bool) -> Result<(), String> {
    let total = Arc::new(AtomicU64::new(0));
    let late_phase = Arc::new(AtomicU64::new(0));
    let sink: VoiceSink = {
        let total = total.clone();
        let late_phase = late_phase.clone();
        Box::new(move |p| {
            if p.data.len() >= 4 {
                let index = u32::from_be_bytes([p.data[0], p.data[1], p.data[2], p.data[3]]);
                total.fetch_add(1, Ordering::Relaxed);
                if index >= 65_536 {
                    late_phase.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
    };
    let mut sender = connect(host, port, "PhishSender", None, log);
    let mut listener = connect(host, port, "PhishListener", Some(sink), log);
    let connected = wait_until(&mut [&mut sender, &mut listener], Duration::from_secs(15), |p| {
        p.iter().all(|x| x.handle.is_connected() && x.handle.own_channel() != 0)
    });
    connected.map_err(|e| format!("connect: {e}"))?;
    std::thread::sleep(Duration::from_millis(500));
    println!("sending {count} voice packets as fast as the link allows (counter wraps at 65536)");
    let start = Instant::now();
    for i in 0..count {
        let mut payload = (i as u32).to_be_bytes().to_vec();
        payload.extend_from_slice(&[0xf8, 0xff, 0xfe, 0x00, 0x00, 0x00, 0x00, 0x00]);
        sender.handle.send_voice(CODEC_OPUS_VOICE, &payload);
        if i % 20 == 19 {
            std::thread::sleep(Duration::from_millis(8));
            sender.pump()?;
            listener.pump()?;
        }
    }
    std::thread::sleep(Duration::from_millis(1500));
    sender.pump()?;
    listener.pump()?;
    let got = total.load(Ordering::Relaxed);
    let after_wrap = late_phase.load(Ordering::Relaxed);
    let expected_after = count.saturating_sub(65_536) as u64;
    println!(
        "  {:.1} s: listener decrypted {got}/{count} packets; {after_wrap}/{expected_after} of those sent after the counter wrapped",
        start.elapsed().as_secs_f32()
    );
    sender.handle.disconnect("done");
    listener.handle.disconnect("done");
    std::thread::sleep(Duration::from_millis(300));
    if got as usize >= count * 9 / 10 && after_wrap >= expected_after * 9 / 10 && expected_after > 0 {
        println!("PASS");
        Ok(())
    } else {
        Err("voice stopped decrypting or was lost".into())
    }
}

fn listen(host: &str, port: u16, seconds: usize, log: bool) -> Result<(), String> {
    let playback = Arc::new(Mutex::new(Playback::new()));
    let counts: Arc<Mutex<std::collections::BTreeMap<u16, (u64, u64)>>> = Arc::new(Mutex::new(Default::default()));
    let sink: VoiceSink = {
        let playback = playback.clone();
        let counts = counts.clone();
        Box::new(move |p| {
            if let Ok(mut c) = counts.lock() {
                let entry = c.entry(p.client_id).or_insert((0, 0));
                if p.data.is_empty() {
                    entry.1 += 1;
                } else {
                    entry.0 += 1;
                }
            }
            if let Ok(mut pb) = playback.lock() {
                pb.push(0, p.client_id, p.voice_id, p.codec, p.data);
            }
        })
    };
    let mut listener = connect(host, port, "PhishListener", Some(sink), log);
    let connected = wait_until(&mut [&mut listener], Duration::from_secs(15), |p| {
        p[0].handle.is_connected() && p[0].handle.own_channel() != 0
    });
    connected.map_err(|e| format!("connect: {e}"))?;
    println!("listening for {seconds} s");
    let mut pcm: Vec<f32> = Vec::new();
    let mut block = vec![0f32; BLOCK];
    let start = Instant::now();
    let mut pulls = 0u64;
    while start.elapsed() < Duration::from_secs(seconds as u64) {
        listener.pump()?;
        let due = start.elapsed().as_millis() as u64 / 20;
        while pulls < due {
            pulls += 1;
            if playback.lock().map(|mut p| p.mix(&mut block)).unwrap_or(0) > 0 {
                pcm.extend_from_slice(&block);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let counts = counts.lock().map(|c| c.clone()).unwrap_or_default();
    for (client, (voice, ends)) in &counts {
        println!("  client {client}: {voice} voice packets, {ends} end-of-talk markers");
    }
    let left: Vec<f32> = pcm.chunks(2).map(|c| c[0]).collect();
    println!(
        "  decoded {:.2} s of audio, rms {:.5}, talk events {} start / {} stop",
        left.len() as f32 / SAMPLE_RATE as f32,
        rms(&left),
        listener.talking_on,
        listener.talking_off
    );
    listener.handle.disconnect("done");
    std::thread::sleep(Duration::from_millis(300));
    if counts.values().any(|(voice, _)| *voice > 0) {
        println!("PASS");
        Ok(())
    } else {
        Err("no voice packets were received".into())
    }
}

fn main() {
    if let Err(e) = run() {
        println!("FAIL: {e}");
        std::process::exit(1);
    }
}
