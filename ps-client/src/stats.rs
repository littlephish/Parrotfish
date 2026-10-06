use std::collections::VecDeque;
use std::time::Instant;

use ps_protocol::command::Command;
use ps_protocol::packet::PacketType;

const KINDS: usize = 3;
const SPEECH: usize = 0;
const KEEPALIVE: usize = 1;
const CONTROL: usize = 2;
const WINDOW_SECONDS: usize = 60;

fn kind(ptype: PacketType) -> usize {
    match ptype {
        PacketType::Voice | PacketType::VoiceWhisper => SPEECH,
        PacketType::Ping | PacketType::Pong => KEEPALIVE,
        _ => CONTROL,
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Bucket {
    second: u64,
    sent: [u64; KINDS],
    received: [u64; KINDS],
}

#[derive(Debug)]
pub struct Stats {
    started: Instant,
    packets_sent: [u64; KINDS],
    packets_received: [u64; KINDS],
    bytes_sent: [u64; KINDS],
    bytes_received: [u64; KINDS],
    buckets: VecDeque<Bucket>,
    pings: VecDeque<f32>,
    pub resent: u64,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            packets_sent: [0; KINDS],
            packets_received: [0; KINDS],
            bytes_sent: [0; KINDS],
            bytes_received: [0; KINDS],
            buckets: VecDeque::new(),
            pings: VecDeque::new(),
            resent: 0,
        }
    }

    fn bucket(&mut self) -> &mut Bucket {
        let second = self.started.elapsed().as_secs();
        if self.buckets.back().map(|b| b.second) != Some(second) {
            self.buckets.push_back(Bucket { second, ..Default::default() });
            while self.buckets.len() > WINDOW_SECONDS + 1 {
                self.buckets.pop_front();
            }
        }
        self.buckets.back_mut().expect("bucket was just pushed")
    }

    pub fn sent(&mut self, ptype: PacketType, len: usize) {
        let k = kind(ptype);
        self.packets_sent[k] += 1;
        self.bytes_sent[k] += len as u64;
        self.bucket().sent[k] += len as u64;
    }

    pub fn received(&mut self, ptype: PacketType, len: usize) {
        let k = kind(ptype);
        self.packets_received[k] += 1;
        self.bytes_received[k] += len as u64;
        self.bucket().received[k] += len as u64;
    }

    pub fn add_ping(&mut self, ms: f32) {
        if self.pings.len() >= 60 {
            self.pings.pop_front();
        }
        self.pings.push_back(ms);
    }

    pub fn ping(&self) -> f32 {
        if self.pings.is_empty() {
            return 0.0;
        }
        self.pings.iter().sum::<f32>() / self.pings.len() as f32
    }

    pub fn ping_deviation(&self) -> f32 {
        if self.pings.len() < 2 {
            return 0.0;
        }
        let avg = self.ping();
        let sum: f32 = self.pings.iter().map(|p| (p - avg) * (p - avg)).sum();
        (sum / (self.pings.len() - 1) as f32).sqrt()
    }

    pub fn voice_in(&self) -> u64 {
        self.packets_received[SPEECH]
    }

    pub fn voice_out(&self) -> u64 {
        self.packets_sent[SPEECH]
    }

    fn bandwidth(&self, seconds: u64, sent: bool) -> [u64; KINDS] {
        let now = self.started.elapsed().as_secs();
        let mut total = [0u64; KINDS];
        for b in &self.buckets {
            if b.second < now && now - b.second <= seconds {
                let src = if sent { &b.sent } else { &b.received };
                for k in 0..KINDS {
                    total[k] += src[k];
                }
            }
        }
        for t in &mut total {
            *t /= seconds.max(1);
        }
        total
    }

    pub fn connection_info(&self) -> Command {
        const NAMES: [&str; KINDS] = ["speech", "keepalive", "control"];
        let mut cmd = Command::new("setconnectioninfo")
            .arg("connection_ping", format!("{:.0}", self.ping()))
            .arg("connection_ping_deviation", format!("{:.4}", self.ping_deviation()));
        for k in 0..KINDS {
            cmd.push(&format!("connection_packets_sent_{}", NAMES[k]), self.packets_sent[k]);
        }
        for k in 0..KINDS {
            cmd.push(&format!("connection_bytes_sent_{}", NAMES[k]), self.bytes_sent[k]);
        }
        for k in 0..KINDS {
            cmd.push(&format!("connection_packets_received_{}", NAMES[k]), self.packets_received[k]);
        }
        for k in 0..KINDS {
            cmd.push(&format!("connection_bytes_received_{}", NAMES[k]), self.bytes_received[k]);
        }
        for name in NAMES.iter().chain(std::iter::once(&"total")) {
            cmd.push(&format!("connection_server2client_packetloss_{name}"), "0.0000");
        }
        let windows: [(&str, u64); 2] = [("second", 1), ("minute", WINDOW_SECONDS as u64)];
        for (direction, sent) in [("sent", true), ("received", false)] {
            for (label, seconds) in windows {
                let values = self.bandwidth(seconds, sent);
                for k in 0..KINDS {
                    cmd.push(
                        &format!("connection_bandwidth_{direction}_last_{label}_{}", NAMES[k]),
                        values[k],
                    );
                }
            }
        }
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_reports() {
        let mut s = Stats::new();
        s.sent(PacketType::Voice, 100);
        s.sent(PacketType::Command, 50);
        s.received(PacketType::Ping, 11);
        s.received(PacketType::VoiceWhisper, 80);
        s.add_ping(10.0);
        s.add_ping(20.0);
        assert_eq!(s.voice_out(), 1);
        assert_eq!(s.voice_in(), 1);
        assert!((s.ping() - 15.0).abs() < 1e-3);
        assert!((s.ping_deviation() - 7.0710678).abs() < 1e-3);
        let text = s.connection_info().build();
        assert!(text.starts_with("setconnectioninfo connection_ping=15 connection_ping_deviation=7.0711 "));
        assert!(text.contains("connection_packets_sent_speech=1 "));
        assert!(text.contains("connection_bytes_sent_control=50 "));
        assert!(text.contains("connection_packets_received_keepalive=1 "));
        assert!(text.contains("connection_bytes_received_speech=80 "));
        assert!(text.contains("connection_server2client_packetloss_total=0.0000"));
        assert!(text.contains("connection_bandwidth_received_last_minute_control=0"));
        assert_eq!(text.matches("connection_").count(), 30);
    }
}
