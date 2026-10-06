use std::fs;
use std::path::PathBuf;

use crate::settings::config_dir;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bookmark {
    pub name: String,
    pub address: String,
    pub nickname: String,
    pub identity_uid: String,
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
            out.push_str(&format!("identity={}\n\n", one_line(&b.identity_uid)));
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
        assert_eq!(initials("172.31.183.111"), "17");
    }
}
