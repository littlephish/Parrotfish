use std::fs;
use std::path::PathBuf;

use crate::platform;
use crate::settings::config_dir;

const SEALED: &str = "dpapi:";

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok()).collect()
}

fn keep(secret: &str) -> Option<String> {
    let sealed = platform::protect(secret.as_bytes())?;
    Some(format!("{SEALED}{}", sealed.iter().map(|byte| format!("{byte:02x}")).collect::<String>()))
}

fn reveal(stored: &str) -> String {
    stored
        .strip_prefix(SEALED)
        .and_then(unhex)
        .and_then(|sealed| platform::unprotect(&sealed))
        .and_then(|plain| String::from_utf8(plain).ok())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bookmark {
    pub name: String,
    pub address: String,
    pub nickname: String,
    pub identity_uid: String,
    pub channel: String,
    pub channel_id: u64,
    pub server_password: String,
    pub channel_password: String,
    pub auto_connect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bookmarks {
    pub items: Vec<Bookmark>,
}

fn bookmarks_path() -> PathBuf {
    config_dir().join("bookmarks.ini")
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

pub fn initials(name: &str) -> String {
    let words: Vec<Vec<char>> = name
        .split_whitespace()
        .map(|word| word.chars().filter(|c| c.is_alphanumeric()).collect::<Vec<char>>())
        .filter(|word| !word.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [] => return "?".to_string(),
        [single] => single.iter().take(2).collect(),
        [first, second, ..] => first.iter().take(1).chain(second.iter().take(1)).collect(),
    };
    letters.to_uppercase()
}

impl Bookmarks {
    pub fn parse(text: &str) -> Self {
        let mut items = Vec::new();
        let mut current: Option<Bookmark> = None;
        let finish = |entry: Option<Bookmark>, items: &mut Vec<Bookmark>| {
            if let Some(mut b) = entry {
                if !b.address.is_empty() {
                    if b.name.is_empty() {
                        b.name = b.address.clone();
                    }
                    items.push(b);
                }
            }
        };
        for raw in text.lines() {
            let line = raw.trim();
            if line.eq_ignore_ascii_case("[bookmark]") {
                finish(current.take(), &mut items);
                current = Some(Bookmark::default());
                continue;
            }
            let Some(entry) = current.as_mut() else {
                continue;
            };
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().to_string();
            match key.trim() {
                "name" => entry.name = value,
                "address" => entry.address = value,
                "nickname" => entry.nickname = value,
                "identity" => entry.identity_uid = value,
                "channel" => entry.channel = value,
                "channel_id" => entry.channel_id = value.parse().unwrap_or(0),
                "server_password" => entry.server_password = reveal(&value),
                "channel_password" => entry.channel_password = reveal(&value),
                "connect_on_start" => entry.auto_connect = value == "1",
                _ => {}
            }
        }
        finish(current.take(), &mut items);
        Self { items }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for b in &self.items {
            out.push_str("[bookmark]\n");
            out.push_str(&format!("name={}\n", one_line(&b.name)));
            out.push_str(&format!("address={}\n", one_line(&b.address)));
            out.push_str(&format!("nickname={}\n", one_line(&b.nickname)));
            out.push_str(&format!("identity={}\n", one_line(&b.identity_uid)));
            out.push_str(&format!("channel={}\n", one_line(&b.channel)));
            out.push_str(&format!("channel_id={}\n", b.channel_id));
            if let Some(sealed) = keep(&b.server_password) {
                out.push_str(&format!("server_password={sealed}\n"));
            }
            if let Some(sealed) = keep(&b.channel_password) {
                out.push_str(&format!("channel_password={sealed}\n"));
            }
            if b.auto_connect {
                out.push_str("connect_on_start=1\n");
            }
            out.push('\n');
        }
        out
    }

    pub fn load() -> Self {
        fs::read_to_string(bookmarks_path()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        fs::create_dir_all(config_dir())?;
        fs::write(bookmarks_path(), self.serialize())
    }

    pub fn find_address(&self, address: &str) -> Option<usize> {
        let wanted = address.trim();
        self.items.iter().position(|b| b.address.trim().eq_ignore_ascii_case(wanted))
    }

    pub fn upsert(&mut self, bookmark: Bookmark) -> usize {
        match self.find_address(&bookmark.address) {
            Some(index) => {
                self.items[index] = bookmark;
                index
            }
            None => {
                self.items.push(bookmark);
                self.items.len() - 1
            }
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bookmark(name: &str, address: &str, nickname: &str, uid: &str) -> Bookmark {
        Bookmark {
            name: name.into(),
            address: address.into(),
            nickname: nickname.into(),
            identity_uid: uid.into(),
            ..Bookmark::default()
        }
    }

    #[test]
    fn round_trip() {
        let list = Bookmarks {
            items: vec![
                bookmark("Reef Runners", "reef.example.net:9988", "LittlePhish", "lks7QL5OVMKo4pZ79cEOI5r5oEA="),
                bookmark("a=b ☺ Zürich", "192.168.1.20", "", ""),
                bookmark("Phish Tank", "[::1]:9987", "Dev Phish", "abc+/=="),
            ],
        };
        assert_eq!(Bookmarks::parse(&list.serialize()), list);
    }

    #[test]
    fn the_channel_to_join_is_kept() {
        let mut home = bookmark("Reef Runners", "reef.example.net", "", "");
        home.channel = "Deep Rock/Radio = loud".into();
        home.channel_id = 4;
        let list = Bookmarks { items: vec![home.clone(), bookmark("Other", "other.example.net", "", "")] };
        let text = list.serialize();
        assert!(text.contains("channel=Deep Rock/Radio = loud\nchannel_id=4\n"));
        assert_eq!(Bookmarks::parse(&text), list);
        let old = Bookmarks::parse("[bookmark]\nname=Old\naddress=old.example.net\nchannel_id=x\n");
        assert_eq!((old.items[0].channel.as_str(), old.items[0].channel_id), ("", 0));
    }

    #[test]
    #[cfg(windows)]
    fn passwords_are_kept_sealed() {
        let mut home = bookmark("Reef Runners", "reef.example.net", "", "");
        home.server_password = "hunter2".into();
        home.channel_password = "tide \u{e4} = pool".into();
        home.auto_connect = true;
        let list = Bookmarks { items: vec![home, bookmark("Other", "other.example.net", "", "")] };
        let text = list.serialize();
        assert!(!text.contains("hunter2") && !text.contains("tide"));
        assert!(text.contains("server_password=dpapi:") && text.contains("channel_password=dpapi:"));
        assert_eq!(text.matches("connect_on_start=1\n").count(), 1);
        assert_eq!(text.matches("password=").count(), 2);
        assert_eq!(Bookmarks::parse(&text), list);
        let tampered = text.replace("server_password=dpapi:", "server_password=dpapi:00");
        let read = Bookmarks::parse(&tampered);
        assert_eq!((read.items[0].server_password.as_str(), read.items[0].channel_password.as_str()), ("", "tide \u{e4} = pool"));
        let plain = Bookmarks::parse(
            "[bookmark]\naddress=a.example.net\nserver_password=hunter2\nchannel_password=dpapi:zz\nconnect_on_start=yes\n",
        );
        assert_eq!((plain.items[0].server_password.as_str(), plain.items[0].channel_password.as_str()), ("", ""));
        assert!(!plain.items[0].auto_connect);
        assert_eq!(unhex("0aFf"), Some(vec![10, 255]));
        assert_eq!((unhex("0"), unhex("zz")), (None, None));
    }

    #[test]
    fn damaged_file_keeps_complete_entries() {
        let text = "garbage before\nname=orphan\n[bookmark]\nname=No address\nnickname=x\n\n[bookmark]\n\n[BOOKMARK]\naddress=ts.example.com\nwhatever\nextra=1\n[bookmark]\nname=Good\naddress=10.0.0.1\nnickname=Me\nidentity=uid=\n";
        let parsed = Bookmarks::parse(text);
        assert_eq!(
            parsed.items,
            vec![bookmark("ts.example.com", "ts.example.com", "", ""), bookmark("Good", "10.0.0.1", "Me", "uid=")]
        );
        assert_eq!(Bookmarks::parse(""), Bookmarks::default());
    }

    #[test]
    fn upsert_replaces_same_address() {
        let mut list = Bookmarks::default();
        assert_eq!(list.upsert(bookmark("A", "TS.Example.com", "one", "")), 0);
        assert_eq!(list.upsert(bookmark("B", "other.example.com", "two", "")), 1);
        assert_eq!(list.upsert(bookmark("A renamed", " ts.example.com ", "three", "")), 0);
        assert_eq!(list.items.len(), 2);
        assert_eq!(list.items[0].name, "A renamed");
        assert_eq!(list.find_address("OTHER.example.com"), Some(1));
        list.remove(0);
        list.remove(7);
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].name, "B");
    }

    #[test]
    fn initials_from_names() {
        assert_eq!(initials("Reef Runners"), "RR");
        assert_eq!(initials("night shift raids"), "NS");
        assert_eq!(initials("Home"), "HO");
        assert_eq!(initials("x"), "X");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("  the   reef "), "TR");
        assert_eq!(initials("ärger über"), "ÄÜ");
        assert_eq!(initials("TeamSpeak ]I[ Server"), "TI");
        assert_eq!(initials("--- ***"), "?");
        assert_eq!(initials("[EU] reef-runners"), "ER");
        assert_eq!(initials("203.0.113.7"), "20");
    }
}
