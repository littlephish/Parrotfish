use std::str::FromStr;

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            ' ' => out.push_str("\\s"),
            '|' => out.push_str("\\p"),
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x0b' => out.push_str("\\v"),
            other => out.push(other),
        }
    }
    out
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('p') => out.push('|'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('v') => out.push('\x0b'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub items: Vec<Vec<(String, String)>>,
}

impl Command {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), items: vec![Vec::new()] }
    }

    pub fn arg(mut self, key: &str, value: impl ToString) -> Self {
        self.push(key, value);
        self
    }

    pub fn flag(mut self, key: &str) -> Self {
        self.push(key, "");
        self
    }

    pub fn push(&mut self, key: &str, value: impl ToString) {
        if self.items.is_empty() {
            self.items.push(Vec::new());
        }
        self.items
            .last_mut()
            .expect("items is not empty")
            .push((key.to_string(), value.to_string()));
    }

    pub fn next_item(mut self) -> Self {
        self.items.push(Vec::new());
        self
    }

    pub fn build(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.name);
        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                out.push('|');
            }
            for (j, (k, v)) in item.iter().enumerate() {
                if j > 0 || (i == 0 && !out.is_empty()) {
                    out.push(' ');
                }
                out.push_str(k);
                if !v.is_empty() {
                    out.push('=');
                    out.push_str(&escape(v));
                }
            }
        }
        out
    }

    pub fn parse(text: &str) -> Self {
        let text = text.trim_matches(|c: char| c == '\0' || c.is_ascii_whitespace());
        let (name, rest) = match text.split_once(' ') {
            Some((first, rest)) if is_name(first) => (first.to_string(), rest),
            None if is_name(text) => (text.to_string(), ""),
            _ => (String::new(), text),
        };
        let mut items = Vec::new();
        for part in rest.split('|') {
            let mut item = Vec::new();
            for token in part.split(|c: char| c == ' ' || c == '\t' || c == '\r' || c == '\n') {
                if token.is_empty() {
                    continue;
                }
                match token.split_once('=') {
                    Some((k, v)) => item.push((k.to_string(), unescape(v))),
                    None => item.push((token.to_string(), String::new())),
                }
            }
            items.push(item);
        }
        Self { name, items }
    }

    pub fn parse_bytes(data: &[u8]) -> Self {
        Self::parse(&String::from_utf8_lossy(data))
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.iter().all(|i| i.is_empty())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.get_at(0, key)
    }

    pub fn get_own(&self, item: usize, key: &str) -> Option<&str> {
        self.items
            .get(item)?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn get_at(&self, item: usize, key: &str) -> Option<&str> {
        self.get_own(item, key).or_else(|| {
            if item == 0 {
                None
            } else {
                self.get_own(0, key)
            }
        })
    }

    pub fn num<T: FromStr>(&self, key: &str) -> Option<T> {
        self.num_at(0, key)
    }

    pub fn num_at<T: FromStr>(&self, item: usize, key: &str) -> Option<T> {
        self.get_at(item, key)?.trim().parse().ok()
    }

    pub fn bool_at(&self, item: usize, key: &str) -> Option<bool> {
        self.get_at(item, key).map(|v| v.trim() == "1")
    }
}

fn is_name(token: &str) -> bool {
    !token.is_empty() && !token.contains('=') && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_round_trip() {
        let raw = "a b|c/d\\e\n\r\t\x07\x08\x0b\x0cé ☺";
        let esc = escape(raw);
        assert_eq!(esc, "a\\sb\\pc\\/d\\\\e\\n\\r\\t\\a\\b\\v\\fé\\s☺");
        assert_eq!(unescape(&esc), raw);
        assert!(!esc.contains(' '));
        assert!(!esc.contains('|'));
    }

    #[test]
    fn builds_like_teamspeak() {
        let cmd = Command::new("clientinit")
            .arg("client_nickname", "Little Phish")
            .arg("client_version", "3.?.? [Build: 5680278000]")
            .flag("client_default_channel")
            .arg("client_key_offset", 213u64);
        assert_eq!(
            cmd.build(),
            "clientinit client_nickname=Little\\sPhish client_version=3.?.?\\s[Build:\\s5680278000] client_default_channel client_key_offset=213"
        );
    }

    #[test]
    fn parses_real_initivexpand2() {
        let text = "initivexpand2 l=AQCVXTlKF+UQc0yga99dOQ9FJCwLaJqtDb1G7xYPMvHFMwIKVfKADF6zAAcAAAAgQW5vbnltb3VzAAAKQo71lhtEMbqAmtuMLlY8Snr0k2Wmymv4hnHNU6tjQCALKHewCykgcA== beta=\\/8kL8lcAYyMJovVOP6MIUC1oZASyuL\\/Y\\/qjVG06R4byuucl9oPAvR7eqZI7z8jGm9jkGmtJ6 omega=MEsDAgcAAgEgAiBxu2eCLQf8zLnuJJ6FtbVjfaOa1210xFgedoXuGzDbTgIgcGk35eqFavKxS4dROi5uKNSNsmzIL4+fyh5Z\\/+FWGxU= ot=1 proof=MEUCIQDRCP4J9e+8IxMJfCLWWI1oIbNPGcChl+3Jr2vIuyDxzAIgOrzRAFPOuJZF4CBw\\/xgbzEsgKMtEtgNobF6WXVNhfUw= tvd time=1544221457";
        let cmd = Command::parse(text);
        assert_eq!(cmd.name, "initivexpand2");
        assert_eq!(cmd.get("ot"), Some("1"));
        assert_eq!(
            cmd.get("beta"),
            Some("/8kL8lcAYyMJovVOP6MIUC1oZASyuL/Y/qjVG06R4byuucl9oPAvR7eqZI7z8jGm9jkGmtJ6")
        );
        assert_eq!(cmd.get("tvd"), Some(""));
        assert_eq!(cmd.num::<u64>("time"), Some(1544221457));
        assert!(cmd.get("omega").unwrap().ends_with("Z/+FWGxU="));
        assert_eq!(cmd.build(), text);
    }

    #[test]
    fn multi_item_inherits_from_first() {
        let cmd = Command::parse(
            "notifyclientmoved ctid=5 reasonid=1 invokerid=3 invokername=Bob\\sX clid=1|clid=2|clid=3 ctid=9",
        );
        assert_eq!(cmd.name, "notifyclientmoved");
        assert_eq!(cmd.len(), 3);
        assert_eq!(cmd.num_at::<u16>(0, "clid"), Some(1));
        assert_eq!(cmd.num_at::<u16>(1, "clid"), Some(2));
        assert_eq!(cmd.num_at::<u64>(1, "ctid"), Some(5));
        assert_eq!(cmd.num_at::<u64>(2, "ctid"), Some(9));
        assert_eq!(cmd.get_at(2, "invokername"), Some("Bob X"));
        assert_eq!(cmd.get_own(2, "invokername"), None);
        assert_eq!(cmd.get_at(1, "missing"), None);
    }

    #[test]
    fn channellist_items() {
        let text = "channellist cid=2 cpid=0 channel_name=Trusted\\sChannel channel_topic channel_codec=0|cid=4 cpid=2 channel_name=Ding\\s•\\s1\\s\\p\\sSplamy´s\\sBett channel_topic channel_codec=4";
        let cmd = Command::parse(text);
        assert_eq!(cmd.len(), 2);
        assert_eq!(cmd.get_at(0, "channel_name"), Some("Trusted Channel"));
        assert_eq!(cmd.get_at(1, "channel_name"), Some("Ding • 1 | Splamy´s Bett"));
        assert_eq!(cmd.num_at::<u8>(1, "channel_codec"), Some(4));
        assert_eq!(cmd.build(), text);
    }

    #[test]
    fn edge_cases() {
        let e = Command::parse("error id=0 msg=ok");
        assert_eq!(e.name, "error");
        assert_eq!(e.num::<u32>("id"), Some(0));

        let bare = Command::parse("channellistfinished");
        assert_eq!(bare.name, "channellistfinished");
        assert!(bare.is_empty());
        assert_eq!(bare.build(), "channellistfinished");

        let nameless = Command::parse("a=1 b=2|a=3");
        assert_eq!(nameless.name, "");
        assert_eq!(nameless.get("a"), Some("1"));
        assert_eq!(nameless.get_at(1, "a"), Some("3"));
        assert_eq!(nameless.build(), "a=1 b=2|a=3");

        let padded = Command::parse_bytes(b"  notifytextmessage  targetmode=2   msg=hi\\sthere \n");
        assert_eq!(padded.name, "notifytextmessage");
        assert_eq!(padded.get("msg"), Some("hi there"));
        assert_eq!(padded.bool_at(0, "targetmode"), Some(false));

        let multi = Command::new("x").arg("a", 1).next_item().arg("a", 2).next_item().arg("a", 3);
        assert_eq!(multi.build(), "x a=1|a=2|a=3");
    }
}
