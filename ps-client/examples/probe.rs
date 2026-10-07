use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ps_client::{ClientHandle, ConnectOptions, Event, ServerView, TextTarget, VoiceSink};
use ps_identity::Identity;

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn arg_values(args: &[String], name: &str) -> Vec<String> {
    args.iter().enumerate().filter(|(_, a)| *a == name).filter_map(|(i, _)| args.get(i + 1)).cloned().collect()
}

fn kind_of(data: &[u8]) -> &'static str {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "PNG"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "GIF"
    } else if data.starts_with(&[0xFF, 0xD8]) {
        "JPEG"
    } else if data.starts_with(b"BM") {
        "BMP"
    } else if data.starts_with(b"RIFF") && data.len() >= 12 && &data[8..12] == b"WEBP" {
        "WebP"
    } else if data.starts_with(&[0, 0, 1, 0]) {
        "ICO"
    } else if data.windows(4).take(256).any(|w| w == b"<svg") {
        "SVG"
    } else {
        "unknown"
    }
}

fn icons_in(view: &ServerView) -> Vec<u32> {
    let mut ids = vec![view.server.icon];
    for node in &view.channels {
        ids.push(node.channel.icon);
        for client in &node.clients {
            ids.extend_from_slice(&client.icons);
        }
    }
    ids.retain(|id| *id != 0 && !ps_client::is_standard_icon(*id));
    ids
}

#[derive(Default)]
struct Stream {
    packets: u32,
    last: Option<Instant>,
    sizes: Vec<usize>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help") {
        eprintln!(
            "usage: probe <host> [--port N] [--identity file.ini | --new-identity] [--nick NAME]\n             [--password PW] [--seconds N] [--say TEXT] [--join CHANNEL_ID] [--log]\n             [--icon ID]... [--all-icons] [--save DIR] [--ft-port N] [--voice] [--token KEY]\n--icon asks for one icon, --all-icons for every icon the server shows; --save keeps the files.\n--voice reports how each talker's stream ends (packet sizes and timing only, no sound)."
        );
        std::process::exit(2);
    }
    let host = args[0].clone();
    let port = arg_value(&args, "--port").and_then(|p| p.parse().ok()).unwrap_or(ps_client::DEFAULT_PORT);
    let seconds: u64 = arg_value(&args, "--seconds").and_then(|p| p.parse().ok()).unwrap_or(5);
    let identity = match arg_value(&args, "--identity") {
        Some(path) => Identity::load(Path::new(&path)).unwrap_or_else(|e| {
            eprintln!("cannot load identity {path}: {e}");
            std::process::exit(2);
        }),
        None => Identity::generate("probe", "Probe"),
    };
    println!("identity uid={} level={} offset={}", identity.uid(), identity.security_level(), identity.key_offset);

    let mut options = ConnectOptions::new(&host, port, identity);
    if let Some(nick) = arg_value(&args, "--nick") {
        options.nickname = nick;
    }
    if let Some(pw) = arg_value(&args, "--password") {
        options.server_password = pw;
    }
    options.log_commands = args.iter().any(|a| a == "--log");
    options.simulated_loss = arg_value(&args, "--loss").and_then(|p| p.parse().ok()).unwrap_or(0.0);
    options.filetransfer_port = arg_value(&args, "--ft-port").and_then(|p| p.parse().ok());
    let channel_password = arg_value(&args, "--channel-password").unwrap_or_default();
    let auto_level = args.iter().any(|a| a == "--auto-level");
    let base_options = options.clone();
    let say = arg_value(&args, "--say");
    let join: Option<u64> = arg_value(&args, "--join").and_then(|p| p.parse().ok());
    let wanted_icons: Vec<u32> = arg_values(&args, "--icon").iter().map(|raw| ps_client::icon_id(raw)).collect();
    let all_icons = args.iter().any(|a| a == "--all-icons");
    let save_to: Option<PathBuf> = arg_value(&args, "--save").map(PathBuf::from);
    let listen = args.iter().any(|a| a == "--voice");
    let privilege_key = arg_value(&args, "--token");

    let voice_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let make_sink = |lines: Arc<Mutex<Vec<String>>>| -> Option<VoiceSink> {
        if !listen {
            return None;
        }
        let mut streams: BTreeMap<u16, Stream> = BTreeMap::new();
        Some(Box::new(move |packet| {
            let stream = streams.entry(packet.client_id).or_default();
            if packet.data.len() <= 1 {
                let gap = stream.last.take().map(|at| at.elapsed().as_millis());
                let tail: Vec<String> = stream.sizes.iter().rev().take(4).rev().map(|n| n.to_string()).collect();
                if let Ok(mut out) = lines.lock() {
                    out.push(format!(
                        "client {} stopped after {} packets with a {}-byte packet{}, last sizes [{}]",
                        packet.client_id,
                        stream.packets,
                        packet.data.len(),
                        gap.map(|ms| format!(" {ms} ms after the last one")).unwrap_or_default(),
                        tail.join(", ")
                    ));
                }
                stream.packets = 0;
                stream.sizes.clear();
            } else {
                if stream.packets == 0 {
                    if let Ok(mut out) = lines.lock() {
                        out.push(format!("client {} started, codec {}", packet.client_id, packet.codec));
                    }
                }
                stream.packets += 1;
                stream.last = Some(Instant::now());
                stream.sizes.push(packet.data.len());
                if stream.sizes.len() > 64 {
                    stream.sizes.drain(..32);
                }
            }
        }))
    };

    let (tx, mut rx) = mpsc::channel();
    let mut handle = ClientHandle::connect(options, tx, make_sink(voice_lines.clone()));
    let mut needed_level: Option<u8> = None;
    let started = Instant::now();
    let mut connected_at: Option<Instant> = None;
    let mut acted = false;
    let mut disconnect_sent = false;
    let mut exit_code = 1;
    let mut asked: HashSet<u32> = HashSet::new();
    let mut answered = 0usize;
    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut last_view: Option<ServerView> = None;

    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Log(text)) => println!("[log] {text}"),
            Ok(Event::State(state)) => println!("[state] {state:?}"),
            Ok(Event::Connected { client_id, server }) => {
                println!(
                    "[connected] client id {client_id} on \"{}\" ({} {}) encryption mode {}, icon {}",
                    server.name, server.platform, server.version, server.codec_encryption_mode, server.icon
                );
                if !server.welcome_message.is_empty() {
                    println!("[welcome] {}", server.welcome_message);
                }
                connected_at = Some(Instant::now());
                exit_code = 0;
            }
            Ok(Event::View(view)) => {
                println!(
                    "[view] {} channels, {} clients, own channel {}, server icon {}",
                    view.channels.len(),
                    view.client_count(),
                    view.own_channel,
                    view.server.icon
                );
                for node in &view.channels {
                    println!(
                        "    {}#{} {} (codec {} q{} {}){}",
                        "  ".repeat(node.depth as usize),
                        node.channel.id,
                        node.channel.name,
                        node.channel.codec,
                        node.channel.codec_quality,
                        if node.channel.unencrypted { "plain" } else { "encrypted" },
                        if node.channel.icon != 0 { format!(" icon {}", node.channel.icon) } else { String::new() }
                    );
                    for c in &node.clients {
                        println!(
                            "    {}  - [{}] {}{}{}",
                            "  ".repeat(node.depth as usize),
                            c.id,
                            c.nickname,
                            if c.id == view.own_id { " (me)" } else { "" },
                            if c.icons.is_empty() { String::new() } else { format!(" icons {:?}", c.icons) }
                        );
                    }
                }
                last_view = Some(view);
            }
            Ok(Event::TextMessage { target, from_name, text, .. }) => {
                println!("[chat {target:?}] <{from_name}> {text}")
            }
            Ok(Event::ServerError { id, message, extra }) => println!("[error {id:#06x}] {message} {extra}"),
            Ok(Event::SecurityLevelRequired(level)) => {
                println!("[security] server requires identity level {level}");
                needed_level = Some(level);
            }
            Ok(Event::Icon { id, data }) => {
                answered += 1;
                match data {
                    Ok(bytes) => {
                        let head: String = bytes.iter().take(8).map(|b| format!("{b:02x}")).collect();
                        let kind = kind_of(&bytes);
                        *kinds.entry(kind).or_default() += 1;
                        println!(
                            "[icon {id}] {} bytes, {kind}, starts {head}, {:.1} s after connecting",
                            bytes.len(),
                            connected_at.map(|at| at.elapsed().as_secs_f32()).unwrap_or(0.0)
                        );
                        if let Some(folder) = &save_to {
                            let written = std::fs::create_dir_all(folder)
                                .and_then(|_| std::fs::write(folder.join(format!("icon_{id}")), &bytes));
                            if let Err(e) = written {
                                println!("[icon {id}] could not be saved: {e}");
                            }
                        }
                    }
                    Err(reason) => {
                        *kinds.entry("failed").or_default() += 1;
                        println!("[icon {id}] failed: {reason}");
                    }
                }
            }
            Ok(Event::Disconnected { reason }) => {
                println!("[disconnected] {reason}");
                if let (true, Some(level)) = (auto_level, needed_level.take()) {
                    let mut retry = base_options.clone();
                    let began = Instant::now();
                    retry.identity.improve_security_level(level, &|| false);
                    println!(
                        "[security] improved identity to level {} (offset {}) in {:.2} s, reconnecting",
                        retry.identity.security_level(),
                        retry.identity.key_offset,
                        began.elapsed().as_secs_f32()
                    );
                    let (tx2, rx2) = mpsc::channel();
                    handle = ClientHandle::connect(retry, tx2, make_sink(voice_lines.clone()));
                    rx = rx2;
                    continue;
                }
                break;
            }
            Ok(Event::Stats(stats)) => println!(
                "[stats] ping {:.1} ms (dev {:.1}), resent {}, voice in/out {}/{}",
                stats.ping_ms, stats.ping_deviation_ms, stats.packets_resent, stats.voice_packets_in, stats.voice_packets_out
            ),
            Ok(other) => println!("[event] {other:?}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if let Ok(mut lines) = voice_lines.lock() {
            for line in lines.drain(..) {
                println!("[voice] {line}");
            }
        }
        if let Some(at) = connected_at {
            if !acted && at.elapsed() > Duration::from_millis(700) {
                acted = true;
                if let Some(channel) = join {
                    handle.join_channel(channel, &channel_password);
                }
                if let Some(text) = &say {
                    handle.send_text(TextTarget::Channel, text);
                    handle.send_text(TextTarget::Server, &format!("{text} (server)"));
                }
                if let Some(key) = &privilege_key {
                    handle.use_privilege_key(key);
                }
                for id in &wanted_icons {
                    if asked.insert(*id) {
                        handle.request_icon(*id);
                    }
                }
            }
            if acted && all_icons {
                if let Some(view) = last_view.take() {
                    for id in icons_in(&view) {
                        if asked.insert(id) {
                            handle.request_icon(id);
                        }
                    }
                }
            }
            if !disconnect_sent && at.elapsed() > Duration::from_secs(seconds) {
                disconnect_sent = true;
                if !asked.is_empty() {
                    let summary: Vec<String> = kinds.iter().map(|(kind, n)| format!("{n} {kind}")).collect();
                    println!("[icons] asked for {}, answered {answered}: {}", asked.len(), summary.join(", "));
                }
                handle.disconnect("probe finished");
            }
        }
        if started.elapsed() > Duration::from_secs(seconds + 30) {
            println!("[probe] giving up");
            exit_code = 1;
            break;
        }
    }
    std::process::exit(exit_code);
}
