use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_WINDOW_WIDTH: f32 = 400.0;
pub const DEFAULT_WINDOW_HEIGHT: f32 = 740.0;
pub const MIN_WINDOW_WIDTH: f32 = 340.0;
pub const MIN_WINDOW_HEIGHT: f32 = 520.0;
const MAX_WINDOW_SIDE: f32 = 8000.0;

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
    pub ptt_key: i32,
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
            ptt_key: 0,
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
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("key_offset.") {
                if let Some((uid, offset)) = rest.rsplit_once('=') {
                    if let Ok(offset) = offset.trim().parse() {
                        s.key_offsets.insert(uid.to_string(), offset);
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
                "ptt_key" => s.ptt_key = value.parse().unwrap_or(0).max(0),
                "window_width" => {
                    s.window_width = number(value, DEFAULT_WINDOW_WIDTH, MIN_WINDOW_WIDTH, MAX_WINDOW_SIDE)
                }
                "window_height" => {
                    s.window_height = number(value, DEFAULT_WINDOW_HEIGHT, MIN_WINDOW_HEIGHT, MAX_WINDOW_SIDE)
                }
                _ => {}
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
        put("ptt_key", self.ptt_key.to_string());
        put("window_width", format!("{:.0}", self.window_width));
        put("window_height", format!("{:.0}", self.window_height));
        for (uid, offset) in &self.key_offsets {
            put(&format!("key_offset.{uid}"), offset.to_string());
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
        s.ptt_key = 3;
        s.window_width = 512.0;
        s.window_height = 900.0;
        s.key_offsets.insert("lks7QL5OVMKo4pZ79cEOI5r5oEA=".into(), 123456);
        let back = Settings::parse(&s.serialize());
        assert_eq!(back, s);
    }

    #[test]
    fn tolerates_garbage_and_clamps() {
        let s = Settings::parse("nonsense\ntx_mode=9\nvad_threshold=abc\nmic_gain=9999\nkey_offset.x=notanumber\n=\n");
        assert_eq!(s.tx_mode, 2);
        assert_eq!(s.vad_threshold, -40.0);
        assert_eq!(s.mic_gain, 300.0);
        assert!(s.key_offsets.is_empty());
        assert_eq!(Settings::parse(""), Settings::default());
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
