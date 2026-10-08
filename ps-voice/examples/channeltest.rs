use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ps_client::{ClientHandle, ConnectOptions, Event, VoiceSink, WhisperGroup, WhisperScope, WhisperTarget};
use ps_identity::Identity;
use ps_oldcodecs::speex;
use ps_voice::codec::{speex_band, Encoder, CODEC_OPUS_VOICE, FRAME_SAMPLES, MAX_PACKET_BYTES, SAMPLE_RATE};
use ps_voice::playback::{Playback, BLOCK};

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
    last: Option<Instant>,
    sizes: BTreeMap<usize, u64>,
    run_started: Option<Instant>,
    spacing_ms: f64,
    spacings: u64,
}

fn read_frames(path: &str) -> Vec<Vec<u8>> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(2);
    });
    let mut frames = Vec::new();
    let mut at = 0usize;
    while at + 2 <= bytes.len() {
        let length = usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
        at += 2;
        if at + length > bytes.len() {
            break;
        }
        frames.push(bytes[at..at + length].to_vec());
        at += length;
    }
    frames
}

fn check_speex(nick: &str, packets: &[(u16, u8, Vec<u8>)], reference: Option<&str>) {
    let Some(&(_, codec, _)) = packets.first() else {
        println!("SPEEX {nick}: no Speex packets were heard");
        return;
    };
    let Some(band) = speex_band(codec) else {
        return;
    };
    let gaps = packets.windows(2).filter(|pair| pair[1].0 != pair[0].0.wrapping_add(1)).count();
    let mut decoder = speex::Decoder::new(band);
    let mut sound = Vec::new();
    let mut refused = 0;
    for (_, _, data) in packets {
        if decoder.decode(data, &mut sound).is_err() {
            refused += 1;
        }
    }
    println!(
        "SPEEX {nick}: {} packets at {} Hz, {gaps} gaps in their numbering, {refused} refused, {} samples decoded",
        packets.len(),
        band.sample_rate(),
        sound.len()
    );
    if let Some(path) = reference {
        match std::fs::read(path) {
            Ok(bytes) => {
                let want: Vec<f32> =
                    bytes.chunks_exact(2).map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]]))).collect();
                let shared = sound.len().min(want.len());
                let worst = sound.iter().zip(&want).map(|(got, want)| (got * 32768.0 - want).abs()).fold(0.0f32, f32::max);
                println!(
                    "SPEEX {nick}: against the reference decoder's sound: {shared} of {} samples compared, largest difference {worst:.2} of 32768",
                    want.len()
                );
            }
            Err(e) => println!("SPEEX {nick}: cannot read {path}: {e}"),
        }
    }
    let mut playback = Playback::new();
    let mut block = vec![0f32; BLOCK];
    let mut played = Vec::new();
    let mut feed = packets.iter();
    for round in 0..packets.len() + 40 {
        if let Some((number, codec, data)) = feed.next() {
            playback.push(1, 1, *number, *codec, data);
            if round + 1 == packets.len() {
                playback.push(1, 1, number.wrapping_add(1), *codec, &[]);
            }
        }
        if playback.mix(&mut block) > 0 {
            played.extend_from_slice(&block);
        }
    }
    let power = played.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / played.len().max(1) as f64;
    let source = sound.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / sound.len().max(1) as f64;
    println!(
        "SPEEX {nick}: through the mixer: {:.2} s at 48 kHz, level {:.1} dBFS (the decoded sound itself: {:.2} s, {:.1} dBFS)",
        played.len() as f64 / 2.0 / 48_000.0,
        10.0 * (power + 1e-12).log10(),
        sound.len() as f64 / f64::from(band.sample_rate()),
        10.0 * (source + 1e-12).log10()
    );
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

fn arg_values(args: &[String], name: &str) -> Vec<String> {
    args.iter().enumerate().filter(|(_, a)| *a == name).filter_map(|(i, _)| args.get(i + 1)).cloned().collect()
}

fn strength_db(samples: &[f32], freq: f32) -> f32 {
    let (mut sin, mut cos) = (0.0f64, 0.0f64);
    for (n, sample) in samples.iter().enumerate() {
        let angle = 2.0 * std::f64::consts::PI * f64::from(freq) * n as f64 / f64::from(SAMPLE_RATE);
        sin += f64::from(*sample) * angle.sin();
        cos += f64::from(*sample) * angle.cos();
    }
    let amplitude = 2.0 * (sin * sin + cos * cos).sqrt() / samples.len().max(1) as f64;
    (20.0 * (amplitude / std::f64::consts::SQRT_2).max(1e-9).log10()) as f32
}

fn level_db(samples: &[f32]) -> f32 {
    let mean = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / samples.len().max(1) as f64;
    (10.0 * mean.max(1e-18).log10()) as f32
}

fn mic_word(word: &str) -> Option<bool> {
    match word {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
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
            "usage: channeltest <host> [--port N] [--nick NAME] [--join CHANNEL_ID] [--seconds N]\n                   [--talk SECONDS] [--talk-after SECONDS]\n                   [--whisper client:ID|channel:ID|commanders|everyone] [--commander] [--abrupt-end]\n                   [--codec N] [--frames FILE] [--frame-ms N] [--save DIR]\n                   [--speex] [--speex-reference FILE] [--mic-off] [--mic SECONDS:on|off]...\n                   [--identity FILE] [--tone HZ] [--amplitude 0..1] [--pulse]\n                   [--mix] [--level] [--lower] [--watch HZ]...\nListens for voice and whispers and reports who was heard; with --talk it also sends a tone, as a whisper when --whisper is given.\n--mic-off signs in with the microphone reported as switched off; --mic tells the server so that many seconds after connecting. Neither stops --talk from sending.\nIt also reports whose microphone it sees switched off or muted, and when that changes.\n--identity keeps the identity in FILE (made if missing), so the server knows the same person again. --tone, --amplitude and --pulse shape what --talk sends; --pulse lets it come and go like speech.\n--mix plays what is heard through the mixer the app uses and reports its level each second, and that of each --watch tone; --level evens people out, --lower follows priority speakers. Nothing is sent to a sound device.\n--codec writes that codec number on what is sent. --frames sends ready-made packets from a file (each one a two-byte length, low byte first, then the bytes), one every --frame-ms.\n--save writes every packet heard to DIR in the same form, one file per talker and codec.\n--speex decodes the Speex packets it heard at the end and reports on them; --speex-reference compares that with a file of 16-bit samples.\nThe end packet follows one frame after the last sound, as in the app; --abrupt-end sends it right behind the last sound,\nwhich a server may deliver the other way round."
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
    let codec: u8 = arg_value(&args, "--codec").and_then(|p| p.parse().ok()).unwrap_or(CODEC_OPUS_VOICE);
    let frames: Vec<Vec<u8>> = arg_value(&args, "--frames").map(|path| read_frames(&path)).unwrap_or_default();
    let frame_ms: f32 = arg_value(&args, "--frame-ms").and_then(|p| p.parse().ok()).unwrap_or(20.0);
    let frame_time = Duration::from_secs_f32(frame_ms.clamp(1.0, 1000.0) / 1000.0);
    let save: Option<PathBuf> = arg_value(&args, "--save").map(PathBuf::from);
    if let Some(dir) = &save {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut saved: BTreeMap<(u16, u8, bool), File> = BTreeMap::new();
    let speex_reference = arg_value(&args, "--speex-reference");
    let speex_wanted = speex_reference.is_some() || args.iter().any(|a| a == "--speex");
    let speex_heard: Arc<Mutex<Vec<(u16, u8, Vec<u8>)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_speex = speex_heard.clone();

    let heard: Arc<Mutex<BTreeMap<u16, Heard>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_heard = heard.clone();
    let sink_lines = lines.clone();
    let mix_wanted = args.iter().any(|a| a == "--mix");
    let lower_wanted = args.iter().any(|a| a == "--lower");
    let watched: Vec<f32> = arg_values(&args, "--watch").iter().filter_map(|hz| hz.parse().ok()).collect();
    let tone_hz: f32 = arg_value(&args, "--tone").and_then(|p| p.parse().ok()).unwrap_or(TONE_HZ);
    let amplitude: f32 = arg_value(&args, "--amplitude").and_then(|p| p.parse().ok()).unwrap_or(AMPLITUDE).clamp(0.0, 1.0);
    let pulse = args.iter().any(|a| a == "--pulse");
    let mixer: Arc<Mutex<Playback>> = Arc::new(Mutex::new(Playback::new()));
    if let Ok(mut mixer) = mixer.lock() {
        mixer.set_leveling(args.iter().any(|a| a == "--level"));
    }
    let sink_mixer = mixer.clone();
    let sink: VoiceSink = Box::new(move |packet| {
        if mix_wanted {
            if let Ok(mut mixer) = sink_mixer.lock() {
                mixer.push_from(0, packet.client_id, packet.voice_id, packet.codec, packet.data, packet.whisper);
            }
        }
        let (Ok(mut map), Ok(mut out)) = (sink_heard.lock(), sink_lines.lock()) else {
            return;
        };
        let entry = map.entry(packet.client_id).or_default();
        let kind = if packet.whisper { "whisper" } else { "voice" };
        if packet.data.len() <= 1 {
            if packet.whisper {
                entry.whisper_ends += 1;
            } else {
                entry.ends += 1;
            }
            let gap = entry
                .last
                .take()
                .map(|at| format!(", {} ms after the last one", at.elapsed().as_millis()))
                .unwrap_or_default();
            out.push(format!(
                "{} end-of-{} packet ({} bytes) from client {} after {} packets{gap}",
                clock(),
                if packet.whisper { "whisper" } else { "talk" },
                packet.data.len(),
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
            if entry.run == 0 {
                entry.run_started = Some(Instant::now());
            } else if let Some(began) = entry.run_started {
                entry.spacing_ms += began.elapsed().as_secs_f64() * 1000.0 / entry.run as f64;
                entry.spacings += 1;
            }
            entry.run += 1;
            entry.last = Some(Instant::now());
            *entry.sizes.entry(packet.data.len()).or_default() += 1;
            if speex_wanted && speex_band(packet.codec).is_some() {
                if let Ok(mut kept) = sink_speex.lock() {
                    kept.push((packet.voice_id, packet.codec, packet.data.to_vec()));
                }
            }
            if let Some(dir) = &save {
                let key = (packet.client_id, packet.codec, packet.whisper);
                let file = saved.entry(key).or_insert_with(|| {
                    let kind = if packet.whisper { "whisper" } else { "voice" };
                    let name = format!("client{}_codec{}_{kind}.bin", packet.client_id, packet.codec);
                    File::create(dir.join(name)).expect("cannot write to the --save folder")
                });
                let length = (packet.data.len().min(usize::from(u16::MAX)) as u16).to_le_bytes();
                let _ = file.write_all(&length);
                let _ = file.write_all(&packet.data[..packet.data.len().min(usize::from(u16::MAX))]);
            }
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

    let mut mic_steps: Vec<(f32, bool)> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "--mic")
        .filter_map(|(i, _)| args.get(i + 1))
        .map(|spec| match spec.split_once(':').and_then(|(at, state)| Some((at.parse().ok()?, mic_word(state)?))) {
            Some(step) => step,
            None => {
                eprintln!("--mic takes SECONDS:on or SECONDS:off");
                std::process::exit(2);
            }
        })
        .collect();
    mic_steps.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut seen_mics: BTreeMap<u16, (bool, bool)> = BTreeMap::new();

    let identity = match arg_value(&args, "--identity").map(PathBuf::from) {
        Some(file) if file.exists() => Identity::load(&file).unwrap_or_else(|e| {
            eprintln!("cannot read the identity in {}: {e}", file.display());
            std::process::exit(2);
        }),
        Some(file) => {
            let made = Identity::generate(&nick, &nick);
            if let Err(e) = made.save(&file) {
                eprintln!("cannot keep the new identity in {}: {e}", file.display());
                std::process::exit(2);
            }
            made
        }
        None => Identity::generate(&nick, &nick),
    };
    let mut priority_seen: BTreeMap<u16, bool> = BTreeMap::new();
    let mut names: BTreeMap<u16, String> = BTreeMap::new();
    let mut next_mix = Instant::now();
    let mut mix_block = vec![0f32; BLOCK];
    let mut second: Vec<f32> = Vec::with_capacity(SAMPLE_RATE as usize);
    let mut options = ConnectOptions::new(&host, port, identity);
    options.nickname = nick.clone();
    options.input_hardware = !args.iter().any(|a| a == "--mic-off");
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
    let abrupt = args.iter().any(|a| a == "--abrupt-end");
    let mut finishing = false;
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
            Ok(Event::View(view)) => {
                for client in view.channels.iter().flat_map(|node| node.clients.iter()).filter(|client| !client.is_query) {
                    let now = (client.input_hardware, client.input_muted);
                    let before = seen_mics.insert(client.id, now);
                    if before.map_or(now != (true, false), |before| before != now) {
                        println!(
                            "{} {nick} sees client {} ({}) with the microphone {}{}",
                            clock(),
                            client.id,
                            client.nickname,
                            if now.0 { "on" } else { "reported off" },
                            if now.1 { " and muted" } else { "" }
                        );
                    }
                    if client.id != view.own_id {
                        names.insert(client.id, client.nickname.clone());
                    }
                    let before = priority_seen.insert(client.id, client.is_priority_speaker);
                    if client.id != view.own_id && before != Some(client.is_priority_speaker) {
                        if client.is_priority_speaker || before == Some(true) {
                            println!(
                                "{} {nick} sees client {} ({}) {} a priority speaker",
                                clock(),
                                client.id,
                                client.nickname,
                                if client.is_priority_speaker { "as" } else { "no longer as" }
                            );
                        }
                        if let Ok(mut mixer) = mixer.lock() {
                            mixer.set_priority(0, client.id, client.is_priority_speaker);
                        }
                    }
                }
                if mix_wanted {
                    let one_myself = view.client(view.own_id).is_some_and(|own| own.is_priority_speaker);
                    let db = view.server.priority_dim_db;
                    if let Ok(mut mixer) = mixer.lock() {
                        mixer.set_priority_dim(0, (lower_wanted && !one_myself && db < 0.0).then_some(db));
                    }
                }
            }
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
            while let Some((_, on)) = mic_steps.first().copied().filter(|(at, _)| since >= *at) {
                mic_steps.remove(0);
                handle.set_input_hardware(on);
                println!("{} {nick} reports the microphone {}", clock(), if on { "on" } else { "off" });
            }
            if !talk_done && since >= talk_after {
                let due = *next_frame.get_or_insert_with(Instant::now);
                if Instant::now() >= due {
                    let loud = !pulse || (phase / FRAME_SAMPLES) % 30 < 20;
                    let scale = if loud { amplitude } else { amplitude * 0.02 };
                    let pcm: Vec<f32> = (0..FRAME_SAMPLES)
                        .map(|i| scale * (2.0 * std::f32::consts::PI * tone_hz * (phase + i) as f32 / SAMPLE_RATE as f32).sin())
                        .collect();
                    phase += FRAME_SAMPLES;
                    let send = |data: &[u8]| match &whisper {
                        Some(target) => handle.send_whisper(target, codec, data),
                        None => handle.send_voice(codec, data),
                    };
                    if finishing {
                        send(&[]);
                    } else if !frames.is_empty() {
                        send(&frames[sent as usize % frames.len()]);
                        sent += 1;
                    } else if let Ok(n) = encoder.encode(&pcm, &mut packet) {
                        send(&packet[..n]);
                        sent += 1;
                    }
                    next_frame = Some(due + if frames.is_empty() { FRAME } else { frame_time });
                    let doing = if whisper.is_some() { "whispering" } else { "talking" };
                    if sent == 1 {
                        println!("{} {nick} starts {doing}", clock());
                    }
                    let each = if frames.is_empty() { 0.02 } else { frame_time.as_secs_f32() };
                    let last = sent as f32 * each >= talk;
                    if last && !finishing && !abrupt {
                        finishing = true;
                    } else if last {
                        if !finishing {
                            send(&[]);
                        }
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

        while mix_wanted && Instant::now() >= next_mix {
            next_mix += FRAME;
            if let Ok(mut mixer) = mixer.lock() {
                mixer.mix(&mut mix_block);
            }
            second.extend(mix_block.chunks(2).map(|frame| frame[0]));
            if second.len() >= SAMPLE_RATE as usize {
                let mut parts = vec![format!("all {:.1} dB", level_db(&second))];
                for hz in &watched {
                    parts.push(format!("{hz:.0} Hz {:.1} dB", strength_db(&second, *hz)));
                }
                if let Ok(mixer) = mixer.lock() {
                    for (client, name) in &names {
                        let heard = mixer.adjustment(0, *client);
                        if heard.leveled_db.abs() >= 0.5 || heard.lowered_db <= -0.5 {
                            parts.push(format!(
                                "{name} levelled {:+.1} dB, lowered {:.1} dB",
                                heard.leveled_db, heard.lowered_db
                            ));
                        }
                    }
                }
                println!("{} {nick} mixed: {}", clock(), parts.join("; "));
                second.clear();
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

    if speex_wanted {
        let packets = speex_heard.lock().map(|kept| kept.clone()).unwrap_or_default();
        check_speex(&nick, &packets, speex_reference.as_deref());
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
        let sizes: Vec<String> = entry.sizes.iter().map(|(bytes, count)| format!("{count} of {bytes} bytes")).collect();
        let spacing = if entry.spacings > 0 { entry.spacing_ms / entry.spacings as f64 } else { 0.0 };
        println!("TOTAL {nick}: client {client}: packet sizes: {}; one packet every {spacing:.1} ms", sizes.join(", "));
    }
}
