use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ps_client::{ClientHandle, ConnectOptions, Event, TextTarget};
use ps_identity::Identity;

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help") {
        eprintln!(
            "usage: probe <host> [--port N] [--identity file.ini | --new-identity] [--nick NAME]\n             [--password PW] [--seconds N] [--say TEXT] [--join CHANNEL_ID] [--log]"
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
    let channel_password = arg_value(&args, "--channel-password").unwrap_or_default();
    let auto_level = args.iter().any(|a| a == "--auto-level");
    let base_options = options.clone();
    let say = arg_value(&args, "--say");
    let join: Option<u64> = arg_value(&args, "--join").and_then(|p| p.parse().ok());

    let (tx, mut rx) = mpsc::channel();
    let mut handle = ClientHandle::connect(options, tx, None);
    let mut needed_level: Option<u8> = None;
    let started = Instant::now();
    let mut connected_at: Option<Instant> = None;
    let mut acted = false;
    let mut disconnect_sent = false;
    let mut exit_code = 1;

    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Log(text)) => println!("[log] {text}"),
            Ok(Event::State(state)) => println!("[state] {state:?}"),
            Ok(Event::Connected { client_id, server }) => {
                println!(
                    "[connected] client id {client_id} on \"{}\" ({} {}) encryption mode {}",
                    server.name, server.platform, server.version, server.codec_encryption_mode
                );
                if !server.welcome_message.is_empty() {
                    println!("[welcome] {}", server.welcome_message);
                }
                connected_at = Some(Instant::now());
                exit_code = 0;
            }
            Ok(Event::View(view)) => {
                println!(
                    "[view] {} channels, {} clients, own channel {}",
                    view.channels.len(),
                    view.client_count(),
                    view.own_channel
                );
                for node in &view.channels {
                    println!(
                        "    {}#{} {} (codec {} q{} {})",
                        "  ".repeat(node.depth as usize),
                        node.channel.id,
                        node.channel.name,
                        node.channel.codec,
                        node.channel.codec_quality,
                        if node.channel.unencrypted { "plain" } else { "encrypted" }
                    );
                    for c in &node.clients {
                        println!(
                            "    {}  - [{}] {}{}",
                            "  ".repeat(node.depth as usize),
                            c.id,
                            c.nickname,
                            if c.id == view.own_id { " (me)" } else { "" }
                        );
                    }
                }
            }
            Ok(Event::TextMessage { target, from_name, text, .. }) => {
                println!("[chat {target:?}] <{from_name}> {text}")
            }
            Ok(Event::ServerError { id, message, extra }) => println!("[error {id:#06x}] {message} {extra}"),
            Ok(Event::SecurityLevelRequired(level)) => {
                println!("[security] server requires identity level {level}");
                needed_level = Some(level);
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
                    handle = ClientHandle::connect(retry, tx2, None);
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
            }
            if !disconnect_sent && at.elapsed() > Duration::from_secs(seconds) {
                disconnect_sent = true;
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
