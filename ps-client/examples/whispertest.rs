use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ps_client::{ClientHandle, ConnectOptions, Event, VoiceSink, WhisperGroup, WhisperScope, WhisperTarget};
use ps_identity::Identity;

struct Peer {
    name: &'static str,
    handle: ClientHandle,
    events: Receiver<Event>,
    heard: Arc<Mutex<(u32, u32)>>,
    id: u16,
}

fn connect(host: &str, name: &'static str) -> Peer {
    let heard = Arc::new(Mutex::new((0u32, 0u32)));
    let counts = heard.clone();
    let sink: VoiceSink = Box::new(move |packet| {
        if let (Ok(mut seen), false) = (counts.lock(), packet.data.is_empty()) {
            if packet.whisper {
                seen.1 += 1;
            } else {
                seen.0 += 1;
            }
        }
    });
    let mut options = ConnectOptions::new(host, ps_client::DEFAULT_PORT, Identity::generate(name, name));
    options.nickname = name.to_string();
    let (tx, rx) = mpsc::channel();
    Peer { name, handle: ClientHandle::connect(options, tx, Some(sink)), events: rx, heard, id: 0 }
}

fn settle(peers: &mut [Peer], time: Duration) {
    let until = Instant::now() + time;
    while Instant::now() < until {
        for peer in peers.iter_mut() {
            while let Ok(event) = peer.events.try_recv() {
                if let Event::Connected { client_id, .. } = event {
                    peer.id = client_id;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn check(peers: &mut [Peer], label: &str, target: Option<&WhisperTarget>, expect: &[&str]) -> bool {
    for peer in peers.iter() {
        *peer.heard.lock().unwrap() = (0, 0);
    }
    for round in 0..16 {
        let data: &[u8] = if round < 15 { &[0x55; 30] } else { &[] };
        match target {
            Some(target) => peers[0].handle.send_whisper(target, 4, data),
            None => peers[0].handle.send_voice(4, data),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(peers, Duration::from_millis(300));
    let got: Vec<&str> = peers
        .iter()
        .skip(1)
        .filter(|peer| {
            let seen = *peer.heard.lock().unwrap();
            if target.is_some() { seen.1 > 0 && seen.0 == 0 } else { seen.0 > 0 && seen.1 == 0 }
        })
        .map(|peer| peer.name)
        .collect();
    let ok = got == expect;
    println!("{} {label}: {got:?}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |name: &str| -> u64 {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(0)
    };
    if args.is_empty() || number("--booth") == 0 || number("--drift") == 0 {
        eprintln!("usage: whispertest <host> --booth CID --drift CID   (run tools/seed_whisper_tree.py first)");
        std::process::exit(2);
    }
    let (lobby, deep, radio, booth, drift) = (1u64, 2u64, 4u64, number("--booth"), number("--drift"));
    let plan = [("Sender", radio), ("InLobby", lobby), ("InDeep", deep), ("InRadio", radio), ("InBooth", booth), ("InDrift", drift)];
    let mut peers: Vec<Peer> = Vec::new();
    for (name, _) in plan.iter() {
        peers.push(connect(&args[0], name));
        settle(&mut peers, Duration::from_millis(400));
    }
    settle(&mut peers, Duration::from_secs(2));
    for (index, (_, channel)) in plan.iter().enumerate() {
        if *channel != lobby {
            peers[index].handle.join_channel(*channel, "");
        }
    }
    settle(&mut peers, Duration::from_millis(1200));
    let drift_id = peers[5].id;
    let everyone = |scope| WhisperTarget::Group { who: WhisperGroup::Everyone, scope };
    let list = |channels: Vec<u64>, clients: Vec<u16>| WhisperTarget::List { channels, clients };
    let mut ok = true;
    ok &= check(&mut peers, "ordinary talk", None, &["InRadio"]);
    ok &= check(&mut peers, "list: Lobby", Some(&list(vec![lobby], vec![])), &["InLobby"]);
    ok &= check(&mut peers, "list: one person", Some(&list(vec![], vec![drift_id])), &["InDrift"]);
    ok &= check(&mut peers, "list: empty", Some(&list(vec![], vec![])), &[]);
    ok &= check(&mut peers, "everyone, everywhere", Some(&everyone(WhisperScope::AllChannels)), &["InLobby", "InDeep", "InRadio", "InBooth", "InDrift"]);
    ok &= check(&mut peers, "everyone, my channel", Some(&everyone(WhisperScope::CurrentChannel)), &["InRadio"]);
    ok &= check(&mut peers, "everyone, the channel above", Some(&everyone(WhisperScope::ParentChannel)), &["InDeep"]);
    ok &= check(&mut peers, "everyone, every channel above", Some(&everyone(WhisperScope::AllParentChannels)), &["InDeep"]);
    ok &= check(&mut peers, "everyone, my channel and below", Some(&everyone(WhisperScope::ChannelFamily)), &["InRadio", "InBooth"]);
    ok &= check(&mut peers, "everyone, my whole branch", Some(&everyone(WhisperScope::WholeFamily)), &["InDeep", "InRadio", "InBooth", "InDrift"]);
    ok &= check(&mut peers, "everyone, right below", Some(&everyone(WhisperScope::Subchannels)), &["InBooth"]);
    let commanders = WhisperTarget::Group { who: WhisperGroup::Commanders, scope: WhisperScope::AllChannels };
    ok &= check(&mut peers, "commanders, everywhere (there are none)", Some(&commanders), &[]);
    for peer in peers.iter() {
        peer.handle.disconnect("whisper test finished");
    }
    settle(&mut peers, Duration::from_millis(1000));
    std::process::exit(if ok { 0 } else { 1 });
}
