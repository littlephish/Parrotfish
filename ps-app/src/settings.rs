use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

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
pub const MAX_PRIORITY_CHANNELS: usize = 64;
pub const MAX_REMEMBERED_VOICES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Voice {
    pub percent: u16,
    pub muted: bool,
    pub unleveled: bool,
    pub priority: bool,
}

impl Voice {
    pub fn plain() -> Self {
        Self { percent: 100, muted: false, unleveled: false, priority: false }
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
    pub check_updates: bool,
    pub updated_from: String,
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
    pub priority_channels: BTreeMap<String, BTreeSet<u64>>,
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
            check_updates: true,
            updated_from: String::new(),
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
            priority_channels: BTreeMap::new(),
            voices: BTreeMap::new(),
            window_width: DEFAULT_WINDOW_WIDTH,
            window_height: DEFAULT_WINDOW_HEIGHT,
            key_offsets: BTreeMap::new(),
        }
    }
}

const FOLDER: &str = "Parrotfish";
const EARLIER_FOLDER: &str = "PhishSpeak";

pub fn profile_root() -> PathBuf {
    std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn folder_in(root: &Path) -> PathBuf {
    let (own, earlier) = (root.join(FOLDER), root.join(EARLIER_FOLDER));
    if earlier.is_dir() && !own.exists() {
        earlier
    } else {
        own
    }
}

pub fn take_over_earlier_folder(root: &Path) -> bool {
    let (own, earlier) = (root.join(FOLDER), root.join(EARLIER_FOLDER));
    earlier.is_dir() && !own.exists() && fs::rename(&earlier, &own).is_ok()
}

pub fn config_dir() -> PathBuf {
    folder_in(&profile_root())
}

pub fn identity_name_now(name: &str) -> String {
    match name.strip_prefix(EARLIER_FOLDER).and_then(|rest| rest.strip_prefix(' ')) {
        Some(number) if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) => format!("{FOLDER} {number}"),
        _ => name.to_string(),
    }
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
                    let flags: Vec<&str> = parts.map(str::trim).collect();
                    let (muted, unleveled, priority) =
                        (flags.contains(&"muted"), flags.contains(&"unleveled"), flags.contains(&"priority"));
                    if let (Some(percent), false) = (percent, uid.is_empty()) {
                        let voice = Voice { percent: percent.min(200), muted, unleveled, priority };
                        if !voice.is_plain() && s.voices.len() < MAX_REMEMBERED_VOICES {
                            s.voices.insert(uid.to_string(), voice);
                        }
                    }
                }
                continue;
            }
            if let Some(rest) = line.trim().strip_prefix("priority_channels.") {
                if let Some((uid, list)) = rest.rsplit_once('=') {
                    let chosen: BTreeSet<u64> =
                        list.split(',').filter_map(|id| id.trim().parse().ok()).take(MAX_PRIORITY_CHANNELS).collect();
                    if !uid.is_empty() && !chosen.is_empty() && s.priority_channels.len() < MAX_REMEMBERED_SERVERS {
                        s.priority_channels.insert(uid.to_string(), chosen);
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
                "check_updates" => s.check_updates = value != "0",
                "updated_from" => s.updated_from = value.chars().filter(|c| c.is_ascii_digit() || *c == '.').take(20).collect(),
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
        put("check_updates", u8::from(self.check_updates).to_string());
        if !self.updated_from.is_empty() {
            put("updated_from", self.updated_from.clone());
        }
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
                let (muted, unleveled) = (if voice.muted { ",muted" } else { "" }, if voice.unleveled { ",unleveled" } else { "" });
                let priority = if voice.priority { ",priority" } else { "" };
                put(&format!("voice.{uid}"), format!("{}{muted}{unleveled}{priority}", voice.percent));
            }
        }
        for (uid, chosen) in &self.priority_channels {
            if !chosen.is_empty() {
                let list: Vec<String> = chosen.iter().map(u64::to_string).collect();
                put(&format!("priority_channels.{uid}"), list.join(","));
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

    pub fn mark_priority_channel(&mut self, server: &str, channel: u64, on: bool, existing: &BTreeSet<u64>, in_use: &[String]) -> bool {
        if server.is_empty() {
            return false;
        }
        let mut chosen = self.priority_channels.remove(server).unwrap_or_default();
        if !existing.is_empty() {
            chosen.retain(|known| existing.contains(known));
        }
        if !on {
            chosen.remove(&channel);
        } else if chosen.len() < MAX_PRIORITY_CHANNELS {
            chosen.insert(channel);
        }
        if chosen.is_empty() {
            return !on;
        }
        if self.priority_channels.len() >= MAX_REMEMBERED_SERVERS {
            let spare = self.priority_channels.keys().find(|known| !in_use.contains(known)).cloned();
            match spare {
                Some(spare) => self.priority_channels.remove(&spare),
                None => return false,
            };
        }
        let kept = chosen.contains(&channel) == on;
        self.priority_channels.insert(server.to_string(), chosen);
        kept
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

    fn scratch_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("parrotfish-profile-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_new_profile_goes_under_the_new_name() {
        let root = scratch_root("fresh");
        assert_eq!(folder_in(&root), root.join("Parrotfish"));
        assert!(!take_over_earlier_folder(&root), "there is nothing to take over");
        assert!(fs::read_dir(&root).unwrap().next().is_none(), "looking creates nothing");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_folder_from_before_the_rename_is_the_profile_until_it_is_moved_whole() {
        let root = scratch_root("moved");
        let (earlier, own) = (root.join("PhishSpeak"), root.join("Parrotfish"));
        fs::create_dir_all(earlier.join("identities")).unwrap();
        fs::write(earlier.join("settings.ini"), "nickname=Minnow\n").unwrap();
        fs::write(earlier.join("identities").join("identity_1.ini"), "kept as it was").unwrap();
        assert_eq!(folder_in(&root), earlier);
        assert!(take_over_earlier_folder(&root));
        assert_eq!(folder_in(&root), own);
        assert!(!earlier.exists(), "moved, not copied");
        assert_eq!(fs::read_to_string(own.join("settings.ini")).unwrap(), "nickname=Minnow\n");
        assert_eq!(fs::read_to_string(own.join("identities").join("identity_1.ini")).unwrap(), "kept as it was");
        assert!(!take_over_earlier_folder(&root), "a second time there is nothing left to move");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_profile_under_the_new_name_is_never_replaced_by_the_earlier_one() {
        let root = scratch_root("both");
        let (earlier, own) = (root.join("PhishSpeak"), root.join("Parrotfish"));
        fs::create_dir_all(&earlier).unwrap();
        fs::create_dir_all(&own).unwrap();
        fs::write(earlier.join("settings.ini"), "nickname=Earlier\n").unwrap();
        fs::write(own.join("settings.ini"), "nickname=Now\n").unwrap();
        assert_eq!(folder_in(&root), own);
        assert!(!take_over_earlier_folder(&root));
        assert_eq!(fs::read_to_string(own.join("settings.ini")).unwrap(), "nickname=Now\n");
        assert_eq!(fs::read_to_string(earlier.join("settings.ini")).unwrap(), "nickname=Earlier\n");
        let _ = fs::remove_dir_all(&root);

        let root = scratch_root("empty");
        fs::create_dir_all(root.join("PhishSpeak")).unwrap();
        fs::write(root.join("PhishSpeak").join("settings.ini"), "nickname=Earlier\n").unwrap();
        fs::create_dir_all(root.join("Parrotfish")).unwrap();
        assert!(!take_over_earlier_folder(&root), "an empty folder under the new name is still the profile");
        assert!(root.join("PhishSpeak").join("settings.ini").is_file());
        assert!(fs::read_dir(root.join("Parrotfish")).unwrap().next().is_none());
        let _ = fs::remove_dir_all(&root);

        let root = scratch_root("file");
        fs::write(root.join("PhishSpeak"), "a file, not a folder").unwrap();
        assert_eq!(folder_in(&root), root.join("Parrotfish"));
        assert!(!take_over_earlier_folder(&root));
        assert!(root.join("PhishSpeak").is_file() && !root.join("Parrotfish").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(windows)]
    fn a_folder_that_cannot_be_moved_yet_is_used_where_it_is() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = scratch_root("busy");
        let (earlier, own) = (root.join("PhishSpeak"), root.join("Parrotfish"));
        fs::create_dir_all(&earlier).unwrap();
        fs::write(earlier.join("settings.ini"), "nickname=Minnow\n").unwrap();
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(earlier.join("settings.ini")).unwrap();
        assert!(!take_over_earlier_folder(&root), "a folder with a file held open was moved");
        assert_eq!(folder_in(&root), earlier);
        assert!(!own.exists());
        drop(held);
        assert!(take_over_earlier_folder(&root), "once the file is free the move goes through");
        assert_eq!(folder_in(&root), own);
        assert_eq!(fs::read_to_string(own.join("settings.ini")).unwrap(), "nickname=Minnow\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn identities_the_earlier_program_named_are_shown_under_the_new_name() {
        assert_eq!(identity_name_now("PhishSpeak 1"), "Parrotfish 1");
        assert_eq!(identity_name_now("PhishSpeak 27"), "Parrotfish 27");
        for chosen in ["PhishSpeak", "PhishSpeak ", "PhishSpeak one", "PhishSpeak 1a", "PhishSpeak  2", "My PhishSpeak 1", "phishspeak 1", "Reef", ""] {
            assert_eq!(identity_name_now(chosen), chosen, "a name somebody chose is left as it is");
        }
    }

    #[test]
    fn who_opens_links_is_remembered() {
        assert_eq!(Settings::default().links, Claim::default());
        let mut s = Settings::default();
        s.links = Claim {
            on: true,
            command: "\"D:\\Apps\\Parrotfish.exe\" \"%1\"".to_string(),
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
    fn looking_for_updates_is_on_until_switched_off_and_an_update_under_way_is_remembered() {
        let plain = Settings::default();
        assert!(plain.check_updates && plain.updated_from.is_empty());
        assert!(plain.serialize().contains("check_updates=1\n") && !plain.serialize().contains("updated_from"));
        let mut s = Settings::default();
        s.check_updates = false;
        s.updated_from = "0.6.0".to_string();
        let text = s.serialize();
        assert!(text.contains("check_updates=0\n") && text.contains("updated_from=0.6.0\n"));
        assert_eq!(Settings::parse(&text), s);
        assert!(Settings::parse("check_updates=maybe\n").check_updates, "anything but a clear no leaves it on");
        assert!(Settings::parse("nickname=Minnow\n").check_updates, "a file from before the setting existed");
        assert_eq!(Settings::parse("updated_from=0.6.0; rm -rf\n").updated_from, "0.6.0");
        assert_eq!(Settings::parse(&format!("updated_from={}\n", "9".repeat(80))).updated_from.len(), 20);
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
    fn the_channels_i_treat_as_priority_are_kept_for_each_server() {
        let mut s = Settings::default();
        s.priority_channels.insert("lks7QL5OVMKo4pZ79cEOI5r5oEA=".into(), BTreeSet::from([4, 17]));
        s.priority_channels.insert("nothing".into(), BTreeSet::new());
        let text = s.serialize();
        assert!(text.contains("priority_channels.lks7QL5OVMKo4pZ79cEOI5r5oEA==4,17\n"));
        assert!(!text.contains("priority_channels.nothing"));
        let back = Settings::parse(&text);
        assert_eq!(back.priority_channels.len(), 1);
        assert_eq!(back.priority_channels["lks7QL5OVMKo4pZ79cEOI5r5oEA="], BTreeSet::from([4, 17]));
        let damaged = Settings::parse("priority_channels.=1\npriority_channels.one=\npriority_channels.two=abc, 7 ,,9x,10\npriority_channels.three\n");
        assert_eq!(damaged.priority_channels.len(), 1);
        assert_eq!(damaged.priority_channels["two"], BTreeSet::from([7, 10]));
        let many: Vec<String> = (1..=500).map(|n| n.to_string()).collect();
        let big = Settings::parse(&format!("priority_channels.big={}\n", many.join(",")));
        assert_eq!(big.priority_channels["big"].len(), MAX_PRIORITY_CHANNELS);
        let crowd: String = (0..200).map(|n| format!("priority_channels.server{n}=1\n")).collect();
        assert_eq!(Settings::parse(&crowd).priority_channels.len(), MAX_REMEMBERED_SERVERS);
    }

    #[test]
    fn marking_a_channel_keeps_within_its_limits_and_says_whether_it_took() {
        let mut s = Settings::default();
        let (all, nobody) = (BTreeSet::new(), Vec::new());
        assert!(!s.mark_priority_channel("", 4, true, &all, &nobody), "a server that has not said who it is");
        assert!(s.priority_channels.is_empty());
        assert!(s.mark_priority_channel("reef=", 4, true, &all, &nobody));
        assert!(s.mark_priority_channel("reef=", 9, true, &all, &nobody));
        assert!(s.mark_priority_channel("reef=", 4, true, &all, &nobody), "ticking twice changes nothing");
        assert_eq!(s.priority_channels["reef="], BTreeSet::from([4, 9]));
        assert!(s.mark_priority_channel("reef=", 4, false, &all, &nobody));
        assert_eq!(s.priority_channels["reef="], BTreeSet::from([9]));
        assert!(s.mark_priority_channel("reef=", 9, false, &all, &nobody));
        assert!(s.priority_channels.is_empty(), "a server with nothing marked is not kept");
        assert!(s.mark_priority_channel("reef=", 77, false, &all, &nobody), "unticking what was never ticked");

        s.priority_channels.insert("reef=".into(), BTreeSet::from([4, 9, 30]));
        let existing = BTreeSet::from([1, 4, 12]);
        assert!(s.mark_priority_channel("reef=", 12, true, &existing, &nobody));
        assert_eq!(s.priority_channels["reef="], BTreeSet::from([4, 12]), "channels that are gone are dropped");

        let full: BTreeSet<u64> = (1..=MAX_PRIORITY_CHANNELS as u64).collect();
        s.priority_channels.insert("big=".into(), full.clone());
        assert!(!s.mark_priority_channel("big=", 900, true, &all, &nobody), "one more than fits is refused");
        assert_eq!(s.priority_channels["big="], full);

        let mut crowded = Settings::default();
        for n in 0..MAX_REMEMBERED_SERVERS {
            crowded.priority_channels.insert(format!("server{n:03}"), BTreeSet::from([1]));
        }
        let busy: Vec<String> = (0..MAX_REMEMBERED_SERVERS).map(|n| format!("server{n:03}")).collect();
        assert!(!crowded.mark_priority_channel("new=", 5, true, &all, &busy), "every remembered server is connected");
        assert!(!crowded.priority_channels.contains_key("new="));
        assert_eq!(crowded.priority_channels.len(), MAX_REMEMBERED_SERVERS);
        assert!(crowded.mark_priority_channel("new=", 5, true, &all, &busy[1..]), "a server that is not connected gives way");
        assert!(crowded.priority_channels.contains_key("new=") && !crowded.priority_channels.contains_key("server000"));
        assert_eq!(crowded.priority_channels.len(), MAX_REMEMBERED_SERVERS);
        assert!(crowded.mark_priority_channel("server005", 8, true, &all, &busy), "a server already remembered needs no room");
        assert_eq!(crowded.priority_channels["server005"], BTreeSet::from([1, 8]));
    }

    #[test]
    fn how_loud_each_person_is_for_me_is_kept() {
        let mut s = Settings::default();
        s.voices.insert("test/9PZ9vww/Bpf5vJxtJhpz80=".into(), Voice { percent: 150, ..Voice::plain() });
        s.voices.insert("lks7QL5OVMKo4pZ79cEOI5r5oEA=".into(), Voice { muted: true, ..Voice::plain() });
        s.voices.insert("asis".into(), Voice { unleveled: true, ..Voice::plain() });
        s.voices.insert("all".into(), Voice { percent: 40, muted: true, unleveled: true, priority: true });
        s.voices.insert("chief".into(), Voice { priority: true, ..Voice::plain() });
        s.voices.insert("plain".into(), Voice::plain());
        let text = s.serialize();
        assert!(text.contains("voice.test/9PZ9vww/Bpf5vJxtJhpz80==150\n"));
        assert!(text.contains("voice.lks7QL5OVMKo4pZ79cEOI5r5oEA==100,muted\n"));
        assert!(!text.contains("voice.plain"));
        assert!(text.contains("voice.asis=100,unleveled\n"));
        assert!(text.contains("voice.all=40,muted,unleveled,priority\n"));
        assert!(text.contains("voice.chief=100,priority\n"));
        let back = Settings::parse(&text);
        assert_eq!(back.voices.len(), 5);
        assert_eq!(back.voices["chief"], Voice { priority: true, ..Voice::plain() });
        assert_eq!(back.voices["test/9PZ9vww/Bpf5vJxtJhpz80="], Voice { percent: 150, ..Voice::plain() });
        assert_eq!(back.voices["asis"], Voice { unleveled: true, ..Voice::plain() });
        assert_eq!(back.voices["all"], Voice { percent: 40, muted: true, unleveled: true, priority: true });
        assert!(back.voices["lks7QL5OVMKo4pZ79cEOI5r5oEA="].muted);
        let odd = Settings::parse(
            "voice.a=900\nvoice.b=x\nvoice.=50\nvoice.c=100\nvoice.d=0, muted \nvoice.e\nvoice.f=100, unleveled , muted\nvoice.g=100,loud\n",
        );
        assert_eq!(odd.voices.len(), 3);
        assert_eq!(odd.voices["a"].percent, 200);
        assert_eq!(odd.voices["d"], Voice { percent: 0, muted: true, ..Voice::plain() });
        assert_eq!(
            odd.voices["f"],
            Voice { muted: true, unleveled: true, ..Voice::plain() },
            "the order of the words does not matter"
        );
        let crowd: String = (0..400).map(|n| format!("voice.person{n}=50\n")).collect();
        assert_eq!(Settings::parse(&crowd).voices.len(), MAX_REMEMBERED_VOICES);
        assert_eq!(Voice::plain().gain(), 1.0);
        assert_eq!(Voice { percent: 50, ..Voice::plain() }.gain(), 0.25);
        assert_eq!(Voice { percent: 200, ..Voice::plain() }.gain(), 4.0);
        assert_eq!(Voice { percent: 200, muted: true, ..Voice::plain() }.gain(), 0.0);
        assert_eq!(Voice { percent: 50, unleveled: true, ..Voice::plain() }.gain(), 0.25);
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
