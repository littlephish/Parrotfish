use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::hotkeys::Chord;
use crate::links::Claim;
use crate::speakers;

const LEGACY_TALK_KEYS: [u16; 17] =
    [0, 0xA2, 0xA3, 0xA4, 0xA5, 0xA0, 0xA1, 0x14, 0x05, 0x06, 0x04, 0xC0, 0x91, 0x13, 0x77, 0x78, 0x79];

pub const DEFAULT_WINDOW_WIDTH: f32 = 400.0;
pub const DEFAULT_WINDOW_HEIGHT: f32 = 740.0;
pub const MIN_WINDOW_WIDTH: f32 = 340.0;
pub const MIN_WINDOW_HEIGHT: f32 = 520.0;
const MAX_WINDOW_SIDE: f32 = 8000.0;
pub const MAX_REMEMBERED_FOLDS: usize = 512;
pub const MAX_REMEMBERED_SERVERS: usize = 64;
pub const MAX_REMEMBERED_VOICES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Voice {
    pub percent: u16,
    pub muted: bool,
}

impl Voice {
    pub fn plain() -> Self {
        Self { percent: 100, muted: false }
    }

    pub fn is_plain(self) -> bool {
        self == Self::plain()
    }

    pub fn gain(self) -> f32 {
        if self.muted {
            return 0.0;
        }
        let level = f32::from(self.percent.min(200)) / 100.0;
        level * level
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub server_address: String,
    pub nickname: String,
    pub identity_path: String,
    pub identity_uid: String,
    pub input_device: String,
    pub output_device: String,
    pub tx_mode: i32,
    pub vad_threshold: f32,
    pub mic_gain: f32,
    pub output_volume: f32,
    pub echo_cancel: bool,
    pub noise_suppression: bool,
    pub auto_gain: bool,
    pub even_voices: bool,
    pub priority_dim: bool,
    pub cue_volume: f32,
    pub talk_keys: Vec<Chord>,
    pub talk_release_ms: u32,
    pub reply_key: Chord,
    pub mute_mic_key: Chord,
    pub mute_sound_key: Chord,
    pub links: Claim,
    pub allow_whispers: bool,
    pub fold_mode: i32,
    pub speakers_shown: bool,
    pub speakers_locked: bool,
    pub speakers_on_top: bool,
    pub speakers_all: bool,
    pub speakers_opacity: f32,
    pub speakers_linger: u32,
    pub speakers_place: Option<(i32, i32)>,
    pub speakers_size: (f32, f32),
    pub folds: BTreeMap<String, BTreeMap<u64, bool>>,
    pub voices: BTreeMap<String, Voice>,
    pub window_width: f32,
    pub window_height: f32,
    pub key_offsets: BTreeMap<String, u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_address: String::new(),
            nickname: String::new(),
            identity_path: String::new(),
            identity_uid: String::new(),
            input_device: String::new(),
            output_device: String::new(),
            tx_mode: 0,
            vad_threshold: -40.0,
            mic_gain: 100.0,
            output_volume: 100.0,
            echo_cancel: false,
            noise_suppression: false,
            auto_gain: false,
            even_voices: false,
            priority_dim: true,
            cue_volume: 50.0,
            talk_keys: Vec::new(),
            talk_release_ms: 0,
            reply_key: Chord::default(),
            mute_mic_key: Chord::default(),
            mute_sound_key: Chord::default(),
            links: Claim::default(),
            allow_whispers: true,
            fold_mode: 1,
            speakers_shown: false,
            speakers_locked: false,
            speakers_on_top: true,
            speakers_all: false,
            speakers_opacity: 85.0,
            speakers_linger: speakers::DEFAULT_LINGER_SECONDS,
            speakers_place: None,
            speakers_size: (220.0, 160.0),
            folds: BTreeMap::new(),
            voices: BTreeMap::new(),
            window_width: DEFAULT_WINDOW_WIDTH,
            window_height: DEFAULT_WINDOW_HEIGHT,
            key_offsets: BTreeMap::new(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("PhishSpeak")
}

pub fn identities_dir() -> PathBuf {
    config_dir().join("identities")
}

fn number(value: &str, fallback: f32, low: f32, high: f32) -> f32 {
    match value.parse::<f32>() {
        Ok(v) if v.is_finite() => v.clamp(low, high),
        _ => fallback,
    }
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.ini")
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        let mut legacy: Option<usize> = None;
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("key_offset.") {
                if let Some((uid, offset)) = rest.rsplit_once('=') {
                    if let Ok(offset) = offset.trim().parse() {
                        s.key_offsets.insert(uid.to_string(), offset);
                    }
                }
                continue;
            }
            if let Some(rest) = line.trim().strip_prefix("voice.") {
                if let Some((uid, value)) = rest.rsplit_once('=') {
                    let mut parts = value.split(',');
                    let percent = parts.next().and_then(|part| part.trim().parse::<u16>().ok());
                    let muted = parts.next().is_some_and(|flag| flag.trim() == "muted");
                    if let (Some(percent), false) = (percent, uid.is_empty()) {
                        let voice = Voice { percent: percent.min(200), muted };
                        if !voice.is_plain() && s.voices.len() < MAX_REMEMBERED_VOICES {
                            s.voices.insert(uid.to_string(), voice);
                        }
                    }
                }
                continue;
            }
            if let Some(rest) = line.trim().strip_prefix("folds.") {
                if let Some((uid, list)) = rest.rsplit_once('=') {
                    let mut chosen = BTreeMap::new();
                    for pair in list.split(',') {
                        let Some((id, state)) = pair.trim().split_once(':') else {
                            continue;
                        };
                        let folded = match state.trim() {
                            "1" => true,
                            "0" => false,
                            _ => continue,
                        };
                        if let (Ok(id), true) = (id.trim().parse::<u64>(), chosen.len() < MAX_REMEMBERED_FOLDS) {
                            chosen.insert(id, folded);
                        }
                    }
                    if !uid.is_empty() && !chosen.is_empty() && s.folds.len() < MAX_REMEMBERED_SERVERS {
                        s.folds.insert(uid.to_string(), chosen);
                    }
                }
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "server_address" => s.server_address = value.to_string(),
                "nickname" => s.nickname = value.to_string(),
                "identity_path" => s.identity_path = value.to_string(),
                "identity_uid" => s.identity_uid = value.to_string(),
                "input_device" => s.input_device = value.to_string(),
                "output_device" => s.output_device = value.to_string(),
                "tx_mode" => s.tx_mode = value.parse().unwrap_or(0).clamp(0, 2),
                "vad_threshold" => s.vad_threshold = number(value, -40.0, -70.0, 0.0),
                "mic_gain" => s.mic_gain = number(value, 100.0, 0.0, 300.0),
                "output_volume" => s.output_volume = number(value, 100.0, 0.0, 200.0),
                "echo_cancel" => s.echo_cancel = value == "1",
                "noise_suppression" => s.noise_suppression = value == "1",
                "auto_gain" => s.auto_gain = value == "1",
                "even_voices" => s.even_voices = value == "1",
                "priority_dim" => s.priority_dim = value != "0",
                "cue_volume" => s.cue_volume = number(value, 50.0, 0.0, 100.0),
                "ptt_key" => legacy = value.parse::<usize>().ok(),
                "talk_key" => {
                    let chord = Chord::parse(value);
                    if !chord.is_empty() {
                        s.talk_keys.push(chord);
                    }
                }
                "talk_release_ms" => s.talk_release_ms = value.parse::<u32>().unwrap_or(0).min(1000),
                "reply_key" => s.reply_key = Chord::parse(value),
                "mute_mic_key" => s.mute_mic_key = Chord::parse(value),
                "mute_sound_key" => s.mute_sound_key = Chord::parse(value),
                "links" => s.links.on = value == "1",
                "links_command" => s.links.command = value.to_string(),
                "links_previous" => s.links.previous = value.to_string(),
                "allow_whispers" => s.allow_whispers = value != "0",
                "fold_mode" => s.fold_mode = value.parse().unwrap_or(1).clamp(0, 2),
                "speakers_shown" => s.speakers_shown = value == "1",
                "speakers_locked" => s.speakers_locked = value == "1",
                "speakers_on_top" => s.speakers_on_top = value != "0",
                "speakers_all" => s.speakers_all = value == "1",
                "speakers_opacity" => s.speakers_opacity = number(value, 85.0, 20.0, 100.0),
                "speakers_linger" => {
                    let seconds = value.parse().unwrap_or(speakers::DEFAULT_LINGER_SECONDS);
                    s.speakers_linger = seconds.min(speakers::MAX_LINGER_SECONDS);
                }
                "speakers_place" => {
                    let parts: Vec<i32> = value.split(',').filter_map(|part| part.trim().parse().ok()).collect();
                    s.speakers_place = if parts.len() == 2 { Some((parts[0], parts[1])) } else { None };
                }
                "speakers_size" => {
                    let parts: Vec<f32> = value.split(',').filter_map(|part| part.trim().parse().ok()).collect();
                    if let [width, height] = parts[..] {
                        s.speakers_size = (
                            width.clamp(speakers::MIN_WIDTH, speakers::MAX_SIDE),
                            height.clamp(speakers::MIN_HEIGHT, speakers::MAX_SIDE),
                        );
                    }
                }
                "window_width" => {
                    s.window_width = number(value, DEFAULT_WINDOW_WIDTH, MIN_WINDOW_WIDTH, MAX_WINDOW_SIDE)
                }
                "window_height" => {
                    s.window_height = number(value, DEFAULT_WINDOW_HEIGHT, MIN_WINDOW_HEIGHT, MAX_WINDOW_SIDE)
                }
                _ => {}
            }
        }
        if s.talk_keys.is_empty() {
            if let Some(code) = legacy.and_then(|index| LEGACY_TALK_KEYS.get(index)).filter(|code| **code != 0) {
                s.talk_keys.push(Chord::new(&[*code]));
            }
        }
        s
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        let mut put = |k: &str, v: String| {
            out.push_str(k);
            out.push('=');
            out.push_str(&v.replace(['\r', '\n'], " "));
            out.push('\n');
        };
        put("server_address", self.server_address.clone());
        put("nickname", self.nickname.clone());
        put("identity_path", self.identity_path.clone());
        put("identity_uid", self.identity_uid.clone());
        put("input_device", self.input_device.clone());
        put("output_device", self.output_device.clone());
        put("tx_mode", self.tx_mode.to_string());
        put("vad_threshold", format!("{:.1}", self.vad_threshold));
        put("mic_gain", format!("{:.0}", self.mic_gain));
        put("output_volume", format!("{:.0}", self.output_volume));
        put("echo_cancel", u8::from(self.echo_cancel).to_string());
        put("noise_suppression", u8::from(self.noise_suppression).to_string());
        put("auto_gain", u8::from(self.auto_gain).to_string());
        put("even_voices", u8::from(self.even_voices).to_string());
        put("priority_dim", u8::from(self.priority_dim).to_string());
        put("cue_volume", format!("{:.0}", self.cue_volume));
        for chord in &self.talk_keys {
            put("talk_key", chord.to_text());
        }
        put("talk_release_ms", self.talk_release_ms.to_string());
        put("reply_key", self.reply_key.to_text());
        put("mute_mic_key", self.mute_mic_key.to_text());
        put("mute_sound_key", self.mute_sound_key.to_text());
        put("links", u8::from(self.links.on).to_string());
        if !self.links.command.is_empty() {
            put("links_command", self.links.command.clone());
        }
        if !self.links.previous.is_empty() {
            put("links_previous", self.links.previous.clone());
        }
        put("allow_whispers", u8::from(self.allow_whispers).to_string());
        put("fold_mode", self.fold_mode.to_string());
        put("speakers_shown", u8::from(self.speakers_shown).to_string());
        put("speakers_locked", u8::from(self.speakers_locked).to_string());
        put("speakers_on_top", u8::from(self.speakers_on_top).to_string());
        put("speakers_all", u8::from(self.speakers_all).to_string());
        put("speakers_opacity", format!("{:.0}", self.speakers_opacity));
        put("speakers_linger", self.speakers_linger.to_string());
        if let Some((x, y)) = self.speakers_place {
            put("speakers_place", format!("{x},{y}"));
        }
        put("speakers_size", format!("{:.0},{:.0}", self.speakers_size.0, self.speakers_size.1));
        put("window_width", format!("{:.0}", self.window_width));
        put("window_height", format!("{:.0}", self.window_height));
        for (uid, offset) in &self.key_offsets {
            put(&format!("key_offset.{uid}"), offset.to_string());
        }
        for (uid, voice) in &self.voices {
            if !voice.is_plain() {
                put(&format!("voice.{uid}"), format!("{}{}", voice.percent, if voice.muted { ",muted" } else { "" }));
            }
        }
        for (uid, chosen) in &self.folds {
            if !chosen.is_empty() {
                let list: Vec<String> =
                    chosen.iter().map(|(id, folded)| format!("{id}:{}", u8::from(*folded))).collect();
                put(&format!("folds.{uid}"), list.join(","));
            }
        }
        out
    }

    pub fn load() -> Self {
        fs::read_to_string(settings_path()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        fs::create_dir_all(config_dir())?;
        fs::write(settings_path(), self.serialize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_opens_links_is_remembered() {
        assert_eq!(Settings::default().links, Claim::default());
        let mut s = Settings::default();
        s.links = Claim {
            on: true,
            command: "\"D:\\Apps\\PhishSpeak.exe\" \"%1\"".to_string(),
            previous: "\"D:\\Other\\voice.exe\" --open=\"%1\"".to_string(),
        };
        let text = s.serialize();
        assert!(text.contains("links=1\n"));
        assert_eq!(Settings::parse(&text), s);
        let plain = Settings::default().serialize();
        assert!(plain.contains("links=0\n") && !plain.contains("links_command") && !plain.contains("links_previous"));
    }

    #[test]
    fn the_speaking_window_is_remembered() {
        let plain = Settings::default();
        assert!(!plain.speakers_shown && !plain.speakers_locked && plain.speakers_on_top && !plain.speakers_all);
        assert_eq!((plain.speakers_opacity, plain.speakers_place, plain.speakers_size), (85.0, None, (220.0, 160.0)));
        assert_eq!(plain.speakers_linger, 10);
        let mut s = Settings::default();
        s.speakers_shown = true;
        s.speakers_locked = true;
        s.speakers_on_top = false;
        s.speakers_all = true;
        s.speakers_opacity = 40.0;
        s.speakers_linger = 0;
        s.speakers_place = Some((-1200, 64));
        s.speakers_size = (300.0, 90.0);
        assert_eq!(Settings::parse(&s.serialize()), s);
        let odd = Settings::parse("speakers_opacity=3\nspeakers_place=7\nspeakers_size=5,x\nspeakers_on_top=maybe\n");
        assert_eq!((odd.speakers_opacity, odd.speakers_place, odd.speakers_size), (20.0, None, (220.0, 160.0)));
        assert!(odd.speakers_on_top);
        assert_eq!(Settings::parse("speakers_linger=25\n").speakers_linger, 25);
        assert_eq!(Settings::parse("speakers_linger=900\n").speakers_linger, 60);
        assert_eq!(Settings::parse("speakers_linger=soon\n").speakers_linger, 10);
        assert_eq!(Settings::parse("speakers_linger=-4\n").speakers_linger, 10);
        assert_eq!(Settings::parse("speakers_size=10,99999\n").speakers_size, (120.0, 2000.0));
        assert_eq!(Settings::parse("speakers_size=300,5\n").speakers_size, (300.0, 38.0));
    }

    #[test]
    fn microphone_helpers_and_event_sounds_are_kept() {
        let plain = Settings::default();
        assert!(!plain.noise_suppression && !plain.auto_gain);
        assert_eq!(plain.cue_volume, 50.0);
        let mut s = Settings::default();
        s.noise_suppression = true;
        s.auto_gain = true;
        s.cue_volume = 0.0;
        let back = Settings::parse(&s.serialize());
        assert!(back.noise_suppression && back.auto_gain);
        assert_eq!(back.cue_volume, 0.0);
        assert_eq!(Settings::parse("cue_volume=900\n").cue_volume, 100.0);
        assert_eq!(Settings::parse("cue_volume=loud\nauto_gain=yes\n"), Settings::default());
    }

    #[test]
    fn the_two_switches_for_other_peoples_voices_are_kept() {
        let plain = Settings::default();
        assert!(!plain.even_voices && plain.priority_dim);
        let mut s = Settings::default();
        s.even_voices = true;
        s.priority_dim = false;
        let back = Settings::parse(&s.serialize());
        assert!(back.even_voices && !back.priority_dim);
        let odd = Settings::parse("even_voices=maybe\npriority_dim=maybe\n");
        assert!(!odd.even_voices && odd.priority_dim);
    }

    #[test]
    fn how_loud_each_person_is_for_me_is_kept() {
        let mut s = Settings::default();
        s.voices.insert("test/9PZ9vww/Bpf5vJxtJhpz80=".into(), Voice { percent: 150, muted: false });
        s.voices.insert("lks7QL5OVMKo4pZ79cEOI5r5oEA=".into(), Voice { percent: 100, muted: true });
        s.voices.insert("plain".into(), Voice::plain());
        let text = s.serialize();
        assert!(text.contains("voice.test/9PZ9vww/Bpf5vJxtJhpz80==150\n"));
        assert!(text.contains("voice.lks7QL5OVMKo4pZ79cEOI5r5oEA==100,muted\n"));
        assert!(!text.contains("voice.plain"));
        let back = Settings::parse(&text);
        assert_eq!(back.voices.len(), 2);
        assert_eq!(back.voices["test/9PZ9vww/Bpf5vJxtJhpz80="], Voice { percent: 150, muted: false });
        assert!(back.voices["lks7QL5OVMKo4pZ79cEOI5r5oEA="].muted);
        let odd = Settings::parse("voice.a=900\nvoice.b=x\nvoice.=50\nvoice.c=100\nvoice.d=0, muted \nvoice.e\n");
        assert_eq!(odd.voices.len(), 2);
        assert_eq!(odd.voices["a"].percent, 200);
        assert_eq!(odd.voices["d"], Voice { percent: 0, muted: true });
        let crowd: String = (0..400).map(|n| format!("voice.person{n}=50\n")).collect();
        assert_eq!(Settings::parse(&crowd).voices.len(), MAX_REMEMBERED_VOICES);
        assert_eq!(Voice::plain().gain(), 1.0);
        assert_eq!(Voice { percent: 50, muted: false }.gain(), 0.25);
        assert_eq!(Voice { percent: 200, muted: false }.gain(), 4.0);
        assert_eq!(Voice { percent: 200, muted: true }.gain(), 0.0);
    }

    #[test]
    fn round_trip() {
        let mut s = Settings::default();
        s.server_address = "ts.example.com:10000".into();
        s.nickname = "Little Phish".into();
        s.identity_uid = "lks7QL5OVMKo4pZ79cEOI5r5oEA=".into();
        s.input_device = "wasapi:{0.0.1.00000000}.{abc}".into();
        s.tx_mode = 1;
        s.vad_threshold = -33.5;
        s.mic_gain = 150.0;
        s.output_volume = 80.0;
        s.echo_cancel = true;
        s.talk_keys = vec![Chord::new(&[0xA4])];
        s.talk_release_ms = 150;
        s.window_width = 512.0;
        s.window_height = 900.0;
        s.key_offsets.insert("lks7QL5OVMKo4pZ79cEOI5r5oEA=".into(), 123456);
        let back = Settings::parse(&s.serialize());
        assert_eq!(back, s);
    }

    #[test]
    fn old_talk_key_setting_is_carried_over() {
        let old = Settings::parse("tx_mode=1\nptt_key=8\n");
        assert_eq!(old.talk_keys, vec![Chord::new(&[0x05])]);
        assert!(Settings::parse("ptt_key=0\n").talk_keys.is_empty());
        assert!(Settings::parse("ptt_key=99\n").talk_keys.is_empty());
        let new = Settings::parse("ptt_key=8\ntalk_key=162+65\ntalk_key=135\ntalk_key=\ntalk_release_ms=250\n");
        assert_eq!(new.talk_keys, vec![Chord::new(&[0xA2, 0x41]), Chord::new(&[0x87])]);
        assert_eq!(new.talk_release_ms, 250);
        let text = new.serialize();
        assert!(text.contains("talk_key=65+162\n") && text.contains("talk_key=135\n") && !text.contains("ptt_key"));
        assert_eq!(Settings::parse(&text), new);
        assert_eq!(Settings::parse("talk_release_ms=99999\n").talk_release_ms, 1000);
        assert_eq!(Settings::default().talk_release_ms, 0);
        let whisper = Settings::parse("reply_key=96\nallow_whispers=0\n");
        assert_eq!(whisper.reply_key, Chord::new(&[0x60]));
        let mutes = Settings::parse("mute_mic_key=162+77\nmute_sound_key=123\n");
        assert_eq!((mutes.mute_mic_key.keys().len(), mutes.mute_sound_key.clone()), (2, Chord::new(&[123])));
        assert_eq!(Settings::parse(&mutes.serialize()), mutes);
        assert!(!whisper.allow_whispers);
        assert_eq!(Settings::parse(&whisper.serialize()), whisper);
        assert!(Settings::default().allow_whispers && Settings::default().reply_key.is_empty());
    }

    #[test]
    fn channel_folding_is_remembered() {
        assert_eq!(Settings::default().fold_mode, 1);
        assert_eq!(Settings::parse("fold_mode=0\n").fold_mode, 0);
        assert_eq!(Settings::parse("fold_mode=2\n").fold_mode, 2);
        assert_eq!(Settings::parse("fold_mode=9\n").fold_mode, 2);
        assert_eq!(Settings::parse("fold_mode=x\n").fold_mode, 1);
        let mut s = Settings::default();
        s.fold_mode = 2;
        s.folds.insert("lks7QL5OVMKo4pZ79cEOI5r5oEA=".into(), BTreeMap::from([(4, true), (17, false)]));
        s.folds.insert("test/9PZ9vww/Bpf5vJxtJhpz80=".into(), BTreeMap::from([(1, false)]));
        s.folds.insert("nothing chosen".into(), BTreeMap::new());
        let text = s.serialize();
        assert!(text.contains("folds.lks7QL5OVMKo4pZ79cEOI5r5oEA==4:1,17:0\n"));
        assert!(!text.contains("nothing chosen"));
        let back = Settings::parse(&text);
        assert_eq!(back.fold_mode, 2);
        assert_eq!(back.folds.len(), 2);
        assert_eq!(back.folds["lks7QL5OVMKo4pZ79cEOI5r5oEA="], BTreeMap::from([(4, true), (17, false)]));
        assert_eq!(back.folds["test/9PZ9vww/Bpf5vJxtJhpz80="], BTreeMap::from([(1, false)]));
        let damaged = Settings::parse("folds.=1:1\nfolds.one=\nfolds.two=abc,7:1,8:x,9, 10 : 0 \nfolds.three\n");
        assert_eq!(damaged.folds.len(), 1);
        assert_eq!(damaged.folds["two"], BTreeMap::from([(7, true), (10, false)]));
        let many: Vec<String> = (0..2000).map(|n| format!("{n}:1")).collect();
        assert_eq!(Settings::parse(&format!("folds.big={}\n", many.join(","))).folds["big"].len(), MAX_REMEMBERED_FOLDS);
        let crowd: String = (0..200).map(|n| format!("folds.server{n}=1:1\n")).collect();
        assert_eq!(Settings::parse(&crowd).folds.len(), MAX_REMEMBERED_SERVERS);
    }

    #[test]
    fn tolerates_garbage_and_clamps() {
        let s = Settings::parse("nonsense\ntx_mode=9\nvad_threshold=abc\nmic_gain=9999\nkey_offset.x=notanumber\n=\n");
        assert_eq!(s.tx_mode, 2);
        assert_eq!(s.vad_threshold, -40.0);
        assert_eq!(s.mic_gain, 300.0);
        assert!(s.key_offsets.is_empty());
        assert_eq!(Settings::parse(""), Settings::default());
        assert!(!Settings::default().echo_cancel && !Settings::parse("echo_cancel=yes\n").echo_cancel);
        assert!(Settings::parse("echo_cancel=1\n").echo_cancel);
        let s = Settings::parse("vad_threshold=NaN\nmic_gain=inf\noutput_volume=-inf\n");
        assert_eq!(s.vad_threshold, -40.0);
        assert_eq!(s.mic_gain, 100.0);
        assert_eq!(s.output_volume, 100.0);
    }

    #[test]
    fn window_size_round_trips_and_clamps() {
        let mut s = Settings::default();
        assert_eq!((s.window_width, s.window_height), (DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT));
        s.window_width = 618.0;
        s.window_height = 1033.0;
        let back = Settings::parse(&s.serialize());
        assert_eq!((back.window_width, back.window_height), (618.0, 1033.0));
        let small = Settings::parse("window_width=12\nwindow_height=-400\n");
        assert_eq!((small.window_width, small.window_height), (MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT));
        let huge = Settings::parse("window_width=999999\nwindow_height=NaN\n");
        assert_eq!((huge.window_width, huge.window_height), (MAX_WINDOW_SIDE, DEFAULT_WINDOW_HEIGHT));
        let junk = Settings::parse("window_width=wide\nwindow_height=\n");
        assert_eq!((junk.window_width, junk.window_height), (DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT));
    }
}
