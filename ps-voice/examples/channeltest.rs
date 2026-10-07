use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ps_client::{ClientHandle, ConnectOptions, Event, VoiceSink, WhisperGroup, WhisperScope, WhisperTarget};
use ps_identity::Identity;
use ps_voice::codec::{Encoder, CODEC_OPUS_VOICE, FRAME_SAMPLES, MAX_PACKET_BYTES, SAMPLE_RATE};

const TONE_HZ: f32 = 440.0;
const AMPLITUDE: f32 = 0.3;
const FRAME: Duration = Duration::from_millis(20);

#[derive(Default, Clone)]
struct Heard {
    packets: u64,
    whispers: u64,
    run: u64,
    run_is_whisper: bool,
    ends: u64,
    whisper_ends: u64,
    codecs: Vec<u8>,
}

fn whisper_target(spec: &str) -> Option<WhisperTarget> {
    let everywhere = |who| WhisperTarget::Group { who, scope: WhisperScope::AllChannels };
    match spec.split_once(':') {
        Some(("client", id)) => id.parse().ok().map(|id| WhisperTarget::List { channels: Vec::new(), clients: vec![id] }),
        Some(("channel", id)) => id.parse().ok().map(|id| WhisperTarget::List { channels: vec![id], clients: Vec::new() }),
        None if spec == "commanders" => Some(everywhere(WhisperGroup::Commanders)),
        None if spec == "everyone" => Some(everywhere(WhisperGroup::Everyone)),
        _ => None,
    }
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn clock() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs() % 86_400;
    format!("{:02}:{:02}:{:02}.{:03}", secs / 3600, (secs / 60) % 60, secs % 60, now.subsec_millis())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help") {
        eprintln!(
            "usage: channeltest <host> [--port N] [--nick NAME] [--join CHANNEL_ID] [--seconds N]\n                   [--talk SECONDS] [--talk-after SECONDS]\n                   [--whisper client:ID|channel:ID|commanders|everyone] [--commander]\nListens for voice and whispers and reports who was heard; with --talk it also sends a tone, as a whisper when --whisper is given."
        );
        std::process::exit(2);
    }
    let host = args[0].clone();
    let port = arg_value(&args, "--port").and_then(|p| p.parse().ok()).unwrap_or(ps_client::DEFAULT_PORT);
    let nick = arg_value(&args, "--nick").unwrap_or_else(|| "ChannelTest".to_string());
    let join: Option<u64> = arg_value(&args, "--join").and_then(|p| p.parse().ok());
    let seconds: f32 = arg_value(&args, "--seconds").and_then(|p| p.parse().ok()).unwrap_or(20.0);
    let talk: f32 = arg_value(&args, "--talk").and_then(|p| p.parse().ok()).unwrap_or(0.0);
    let talk_after: f32 = arg_value(&args, "--talk-after").and_then(|p| p.parse().ok()).unwrap_or(1.5);
    let whisper = arg_value(&args, "--whisper").map(|spec| match whisper_target(&spec) {
        Some(target) => target,
        None => {
            eprintln!("--whisper takes client:ID, channel:ID, commanders or everyone");
            std::process::exit(2);
        }
    });
    let commander = args.iter().any(|a| a == "--commander");

    let heard: Arc<Mutex<BTreeMap<u16, Heard>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_heard = heard.clone();
    let sink_lines = lines.clone();
    let sink: VoiceSink = Box::new(move |packet| {
        let (Ok(mut map), Ok(mut out)) = (sink_heard.lock(), sink_lines.lock()) else {
            return;
        };
        let entry = map.entry(packet.client_id).or_default();
        let kind = if packet.whisper { "whisper" } else { "voice" };
        if packet.data.is_empty() {
            if packet.whisper {
                entry.whisper_ends += 1;
            } else {
                entry.ends += 1;
            }
            out.push(format!(
                "{} end-of-{} packet from client {} after {} packets",
                clock(),
                if packet.whisper { "whisper" } else { "talk" },
                packet.client_id,
                entry.run
            ));
            entry.run = 0;
        } else {
            if entry.run == 0 || entry.run_is_whisper != packet.whisper {
                out.push(format!("{} first {kind} packet from client {}", clock(), packet.client_id));
                entry.run = 0;
            }
            entry.run_is_whisper = packet.whisper;
            entry.run += 1;
            if packet.whisper {
                entry.whispers += 1;
            } else {
                entry.packets += 1;
            }
            if !entry.codecs.contains(&packet.codec) {
                entry.codecs.push(packet.codec);
            }
        }
    });

    let mut options = ConnectOptions::new(&host, port, Identity::generate(&nick, &nick));
    options.nickname = nick.clone();
    let (tx, rx) = mpsc::channel();
    let handle = ClientHandle::connect(options, tx, Some(sink));

    let mut encoder = match Encoder::new(CODEC_OPUS_VOICE, 6) {
        Ok(encoder) => encoder,
        Err(e) => {
            eprintln!("cannot create the encoder: {e}");
            std::process::exit(2);
        }
    };
    let mut packet = [0u8; MAX_PACKET_BYTES];
    let mut phase = 0usize;
    let mut connected_at: Option<Instant> = None;
    let mut joined = false;
    let mut next_frame: Option<Instant> = None;
    let mut sent = 0u64;
    let mut talk_done = talk <= 0.0;
    let mut last_report = Instant::now();
    let mut last_counts: BTreeMap<u16, (u64, u64)> = BTreeMap::new();
    let mut leaving = false;
    let started = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(2)) {
            Ok(Event::Connected { client_id, server }) => {
                println!("{} {nick} connected as client {client_id} to \"{}\"", clock(), server.name);
                connected_at = Some(Instant::now());
            }
            Ok(Event::ClientMoved { client, to, .. }) => {
                if client.id == handle.client_id() {
                    println!("{} {nick} is now in channel {to}", clock());
                }
            }
            Ok(Event::ServerError { id, message, extra }) => println!("{} server error {id:#06x}: {message} {extra}", clock()),
            Ok(Event::Disconnected { reason }) => {
                println!("{} {nick} disconnected: {reason}", clock());
                break;
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if let Ok(mut out) = lines.lock() {
            for line in out.drain(..) {
                println!("{line}");
            }
        }

        if let Some(at) = connected_at {
            let since = at.elapsed().as_secs_f32();
            if !joined && since > 0.7 {
                joined = true;
                if let Some(channel) = join {
                    handle.join_channel(channel, "");
                }
                if commander {
                    handle.set_channel_commander(true);
                }
            }
            if !talk_done && since >= talk_after {
                let due = *next_frame.get_or_insert_with(Instant::now);
                if Instant::now() >= due {
                    let pcm: Vec<f32> = (0..FRAME_SAMPLES)
                        .map(|i| {
                            AMPLITUDE
                                * (2.0 * std::f32::consts::PI * TONE_HZ * (phase + i) as f32 / SAMPLE_RATE as f32).sin()
                        })
                        .collect();
                    phase += FRAME_SAMPLES;
                    let send = |data: &[u8]| match &whisper {
                        Some(target) => handle.send_whisper(target, CODEC_OPUS_VOICE, data),
                        None => handle.send_voice(CODEC_OPUS_VOICE, data),
                    };
                    if let Ok(n) = encoder.encode(&pcm, &mut packet) {
                        send(&packet[..n]);
                        sent += 1;
                    }
                    next_frame = Some(due + FRAME);
                    let doing = if whisper.is_some() { "whispering" } else { "talking" };
                    if sent == 1 {
                        println!("{} {nick} starts {doing}", clock());
                    }
                    if sent as f32 * 0.02 >= talk {
                        send(&[]);
                        talk_done = true;
                        println!("{} {nick} stops {doing} after {sent} packets", clock());
                    }
                }
            }
            if !leaving && since >= seconds {
                leaving = true;
                handle.disconnect("channel test finished");
            }
        }

        if last_report.elapsed() >= Duration::from_secs(1) {
            last_report = Instant::now();
            if let Ok(map) = heard.lock() {
                let mut parts = Vec::new();
                for (client, entry) in map.iter() {
                    let now = (entry.packets, entry.whispers);
                    let before = last_counts.insert(*client, now).unwrap_or((0, 0));
                    if now != before {
                        parts.push(format!(
                            "client {client}: +{} voice, +{} whisper",
                            now.0 - before.0,
                            now.1 - before.1
                        ));
                    }
                }
                if !parts.is_empty() {
                    println!("{} {nick} heard in the last second: {}", clock(), parts.join(", "));
                }
            }
        }

        if started.elapsed().as_secs_f32() > seconds + 30.0 {
            println!("{} giving up", clock());
            break;
        }
    }

    let totals: BTreeMap<u16, Heard> = heard.lock().map(|map| map.clone()).unwrap_or_default();
    if totals.is_empty() {
        println!("TOTAL {nick}: heard nobody");
    }
    for (client, entry) in &totals {
        println!(
            "TOTAL {nick}: client {client}: {} voice packets, {} end-of-talk packets, {} whisper packets, {} end-of-whisper packets, codecs {:?}",
            entry.packets, entry.ends, entry.whispers, entry.whisper_ends, entry.codecs
        );
    }
}
