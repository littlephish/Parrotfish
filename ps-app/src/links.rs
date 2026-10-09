pub const SCHEME: &str = "ts3server";
const MAX_LINK: usize = 2048;
const MAX_HOST: usize = 253;
const MAX_SECRET: usize = 256;
const MAX_CHANNEL: usize = 512;
const MAX_LABEL: usize = 64;
const MAX_SCHEME: usize = 32;
const NICKNAME_CHARS: std::ops::RangeInclusive<usize> = 3..=30;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Link {
    pub address: String,
    pub nickname: String,
    pub password: String,
    pub channel: String,
    pub channel_id: u64,
    pub channel_password: String,
    pub token: String,
    pub bookmark: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    NotALink,
    NoServer,
    TooLong,
    Unreadable,
}

fn valid_scheme(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_SCHEME
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '+'))
}

pub fn scheme_from(chosen: Option<&str>) -> String {
    chosen.map(str::trim).filter(|name| valid_scheme(name)).unwrap_or(SCHEME).to_ascii_lowercase()
}

pub fn scheme() -> String {
    scheme_from(std::env::var("PARROTFISH_LINK_SCHEME").ok().as_deref())
}

fn after_scheme<'a>(text: &'a str, scheme: &str) -> Option<&'a str> {
    let (name, rest) = text.split_once("://")?;
    (name.eq_ignore_ascii_case(scheme) || name.eq_ignore_ascii_case(SCHEME)).then_some(rest)
}

pub fn is_link(text: &str, scheme: &str) -> bool {
    after_scheme(text.trim(), scheme).is_some()
}

fn unescape(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let pair = bytes.get(at + 1..at + 3)?;
            if !pair.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            out.push((high * 16 + low) as u8);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    let text = String::from_utf8(out).ok()?;
    (!text.chars().any(char::is_control)).then_some(text)
}

fn port_number(text: &str) -> Result<u16, Refused> {
    match text.parse::<u16>() {
        Ok(port) if port != 0 && text.bytes().all(|b| b.is_ascii_digit()) => Ok(port),
        _ => Err(Refused::Unreadable),
    }
}

fn host_and_port(authority: &str) -> Result<(String, Option<u16>), Refused> {
    if authority.is_empty() {
        return Err(Refused::NoServer);
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let (inside, after) = rest.split_once(']').ok_or(Refused::Unreadable)?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port_number(port)?),
            None if after.is_empty() => None,
            None => return Err(Refused::Unreadable),
        };
        let good = !inside.is_empty()
            && inside.len() <= 45
            && inside.contains(':')
            && inside.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
        return if good { Ok((format!("[{inside}]"), port)) } else { Err(Refused::Unreadable) };
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port_number(port)?)),
        None => (authority, None),
    };
    if host.is_empty() {
        return Err(Refused::NoServer);
    }
    let good = host.chars().count() <= MAX_HOST
        && host.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_'))
        && !host.starts_with(['-', '.'])
        && !host.contains("..");
    if good {
        Ok((host.to_string(), port))
    } else {
        Err(Refused::Unreadable)
    }
}

fn secret(value: &str) -> Result<String, Refused> {
    if value.chars().count() <= MAX_SECRET {
        Ok(value.to_string())
    } else {
        Err(Refused::Unreadable)
    }
}

pub fn parse(text: &str, scheme: &str) -> Result<Link, Refused> {
    let text = text.trim();
    let rest = after_scheme(text, scheme).ok_or(Refused::NotALink)?;
    if text.len() > MAX_LINK {
        return Err(Refused::TooLong);
    }
    if text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(Refused::Unreadable);
    }
    let rest = rest.split('#').next().unwrap_or("");
    let (place, query) = rest.split_once('?').unwrap_or((rest, ""));
    let authority = place.split('/').next().unwrap_or("");
    if authority.contains(['@', '\\', '%']) {
        return Err(Refused::Unreadable);
    }
    let (host, mut port) = host_and_port(authority)?;
    let mut link = Link::default();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = unescape(value).ok_or(Refused::Unreadable)?;
        match key.to_ascii_lowercase().as_str() {
            "port" => port = Some(port_number(&value)?),
            "nickname" => {
                let name = value.trim();
                link.nickname =
                    if NICKNAME_CHARS.contains(&name.chars().count()) { name.to_string() } else { String::new() };
            }
            "password" => link.password = secret(&value)?,
            "channelpassword" => link.channel_password = secret(&value)?,
            "token" => link.token = secret(value.trim())?,
            "channel" => {
                if value.chars().count() > MAX_CHANNEL {
                    return Err(Refused::Unreadable);
                }
                link.channel = value.trim().to_string();
            }
            "cid" => link.channel_id = value.trim().parse().unwrap_or(0),
            "addbookmark" => link.bookmark = value.trim().chars().take(MAX_LABEL).collect::<String>().trim().to_string(),
            _ => {}
        }
    }
    if link.channel_id != 0 {
        link.channel.clear();
    }
    if link.channel.is_empty() && link.channel_id == 0 {
        link.channel_password.clear();
    }
    link.address = match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    Ok(link)
}

pub fn summary(link: &Link) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !link.password.is_empty() {
        parts.push("brings a server password".to_string());
    }
    if link.channel_id != 0 {
        parts.push("goes to a channel it names by number".to_string());
    } else if !link.channel.is_empty() {
        parts.push(format!("goes to the channel {}", link.channel));
    }
    if !link.channel_password.is_empty() {
        parts.push("brings a password for that channel".to_string());
    }
    if !link.token.is_empty() {
        parts.push("uses a privilege key".to_string());
    }
    if !link.bookmark.is_empty() {
        parts.push(format!("wants to be saved as the bookmark {}", link.bookmark));
    }
    let also = match parts.len() {
        0 => String::new(),
        1 => format!(" It also {}.", parts[0]),
        _ => {
            let last = parts.pop().unwrap_or_default();
            format!(" It also {} and {last}.", parts.join(", "))
        }
    };
    format!("This came from a link. Check the address before you connect.{also}")
}

pub trait Handlers {
    fn current(&self) -> Option<String>;
    fn set(&mut self, command: &str) -> bool;
    fn clear(&mut self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Claim {
    pub on: bool,
    pub command: String,
    pub previous: String,
}

pub fn command_for(program: &str) -> String {
    format!("\"{program}\" \"%1\"")
}

fn same(a: &str, b: &str) -> bool {
    !a.is_empty() && a.eq_ignore_ascii_case(b)
}

pub fn claim(handlers: &mut dyn Handlers, program: &str, state: &mut Claim) -> bool {
    let wanted = command_for(program);
    let now = handlers.current();
    if !now.as_deref().is_some_and(|command| same(command, &wanted)) {
        let theirs = now.filter(|command| !same(command, &state.command));
        if !handlers.set(&wanted) {
            return false;
        }
        if let Some(theirs) = theirs {
            state.previous = theirs;
        }
    }
    state.on = true;
    state.command = wanted;
    true
}

pub fn release(handlers: &mut dyn Handlers, state: &mut Claim) -> bool {
    let ours = handlers.current().is_some_and(|command| same(&command, &state.command));
    let done = if !ours {
        true
    } else if state.previous.is_empty() {
        handlers.clear()
    } else {
        handlers.set(&state.previous)
    };
    if done {
        *state = Claim::default();
    }
    done
}

pub fn refresh(handlers: &mut dyn Handlers, program: &str, state: &mut Claim) {
    if !state.on {
        return;
    }
    let wanted = command_for(program);
    match handlers.current() {
        Some(now) if same(&now, &wanted) => state.command = wanted,
        Some(now) if !same(&now, &state.command) => *state = Claim::default(),
        _ => {
            if handlers.set(&wanted) {
                state.command = wanted;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Link, Refused> {
        parse(text, SCHEME)
    }

    #[test]
    fn the_documented_link_with_everything_in_it_is_read() {
        let link = read(
            "ts3server://ts3.hoster.com?port=9987&nickname=UserNickname&password=serverPassword&channel=MyDefaultChannel&cid=42&channelpassword=defaultChannelPassword&token=TokenKey&addbookmark=MyBookMarkLabel",
        )
        .unwrap();
        assert_eq!(link.address, "ts3.hoster.com:9987");
        assert_eq!(link.nickname, "UserNickname");
        assert_eq!(link.password, "serverPassword");
        assert_eq!(link.channel, "", "a channel number wins over a channel name");
        assert_eq!(link.channel_id, 42);
        assert_eq!(link.channel_password, "defaultChannelPassword");
        assert_eq!(link.token, "TokenKey");
        assert_eq!(link.bookmark, "MyBookMarkLabel");
    }

    #[test]
    fn the_short_forms_are_read() {
        assert_eq!(read("ts3server://ts3.hoster.com").unwrap(), Link { address: "ts3.hoster.com".into(), ..Link::default() });
        assert_eq!(read("ts3server://ts3.hoster.com:9988").unwrap().address, "ts3.hoster.com:9988");
        assert_eq!(read("ts3server://ts3.hoster.com?port=9988").unwrap().address, "ts3.hoster.com:9988");
        assert_eq!(read("ts3server://ts3.hoster.com:1?port=2").unwrap().address, "ts3.hoster.com:2", "the named port wins");
        assert_eq!(read("  TS3Server://Voice.Example.org/  ").unwrap().address, "Voice.Example.org");
        assert_eq!(read("ts3server://203.0.113.9:10000/?").unwrap().address, "203.0.113.9:10000");
        assert_eq!(read("ts3server://[2001:db8::7]:9987").unwrap().address, "[2001:db8::7]:9987");
        assert_eq!(read("ts3server://[2001:db8::7]").unwrap().address, "[2001:db8::7]");
        assert_eq!(read("ts3server://voice.example.org#fragment").unwrap().address, "voice.example.org");
        assert_eq!(read("ts3server://m\u{fc}nchen.example").unwrap().address, "m\u{fc}nchen.example");
    }

    #[test]
    fn escaped_text_is_unescaped_and_a_plus_stays_a_plus() {
        let link = read("ts3server://example.org?nickname=Web%20Guest&channel=Lobby%2FSide%20Room&password=a%26b%3Dc+d").unwrap();
        assert_eq!(link.nickname, "Web Guest");
        assert_eq!(link.channel, "Lobby/Side Room");
        assert_eq!(link.password, "a&b=c+d");
        let link = read("ts3server://example.org?NickName=Fr%C3%A9d%C3%A9ric&unknown=1&&channel=").unwrap();
        assert_eq!(link.nickname, "Fr\u{e9}d\u{e9}ric");
        assert_eq!(link.channel, "");
    }

    #[test]
    fn details_that_make_no_sense_are_dropped_and_the_rest_is_kept() {
        let link = read("ts3server://example.org?nickname=ab&cid=seven&channelpassword=x&addbookmark=%20%20").unwrap();
        assert_eq!(link, Link { address: "example.org".into(), ..Link::default() });
        let long = "n".repeat(31);
        assert_eq!(read(&format!("ts3server://example.org?nickname={long}")).unwrap().nickname, "");
        let label = "b".repeat(200);
        assert_eq!(read(&format!("ts3server://example.org?addbookmark={label}")).unwrap().bookmark.len(), 64);
        let link = read("ts3server://example.org?channel=Lobby&channelpassword=pw&cid=0").unwrap();
        assert_eq!((link.channel.as_str(), link.channel_password.as_str()), ("Lobby", "pw"));
    }

    #[test]
    fn links_that_could_mislead_are_refused() {
        assert_eq!(read("http://example.org"), Err(Refused::NotALink));
        assert_eq!(read("ts3server:example.org"), Err(Refused::NotALink));
        assert_eq!(read("example.org"), Err(Refused::NotALink));
        assert_eq!(read("ts3server://"), Err(Refused::NoServer));
        assert_eq!(read("ts3server://?port=9987"), Err(Refused::NoServer));
        assert_eq!(read("ts3server://:9987"), Err(Refused::NoServer));
        assert_eq!(read("ts3server://good.example@evil.example"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://evil.example%2Fgood.example"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org\\other"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://exa mple.org"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org:99999"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org:0"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org:+80"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org?port=abc"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org?nickname=%0D%0Aname"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org?nickname=%ZZ"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org?nickname=%4"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org?nickname=%+1abc"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://example.org?nickname=%FF%FE"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://::1"), Err(Refused::Unreadable), "an IPv6 address needs its brackets");
        assert_eq!(read("ts3server://[example]"), Err(Refused::Unreadable));
        assert_eq!(read("ts3server://..example.org"), Err(Refused::Unreadable));
        let long = "p".repeat(257);
        assert_eq!(read(&format!("ts3server://example.org?password={long}")), Err(Refused::Unreadable));
        let huge = "c".repeat(3000);
        assert_eq!(read(&format!("ts3server://example.org?channel={huge}")), Err(Refused::TooLong));
    }

    #[test]
    fn another_scheme_can_stand_in_for_tests() {
        assert_eq!(scheme_from(None), "ts3server");
        assert_eq!(scheme_from(Some("TS3Server-Test")), "ts3server-test");
        assert_eq!(scheme_from(Some("9lives")), "ts3server");
        assert_eq!(scheme_from(Some("bad scheme")), "ts3server");
        assert_eq!(scheme_from(Some("")), "ts3server");
        assert!(is_link("ts3server-test://example.org", "ts3server-test"));
        assert!(is_link("ts3server://example.org", "ts3server-test"), "the real one always counts");
        assert!(!is_link("ts3server-test://example.org", "ts3server"));
        assert!(!is_link("--connect", "ts3server"));
        assert_eq!(parse("ts3server-test://example.org:5", "ts3server-test").unwrap().address, "example.org:5");
    }

    #[test]
    fn the_note_says_what_a_link_brings_along() {
        let plain = read("ts3server://example.org").unwrap();
        assert_eq!(summary(&plain), "This came from a link. Check the address before you connect.");
        let one = read("ts3server://example.org?token=abc").unwrap();
        assert_eq!(summary(&one), "This came from a link. Check the address before you connect. It also uses a privilege key.");
        let all = read("ts3server://example.org?password=a&channel=Lobby&channelpassword=b&token=c&addbookmark=Reef").unwrap();
        assert_eq!(
            summary(&all),
            "This came from a link. Check the address before you connect. It also brings a server password, goes to the channel Lobby, brings a password for that channel, uses a privilege key and wants to be saved as the bookmark Reef."
        );
        let numbered = read("ts3server://example.org?cid=7").unwrap();
        assert!(summary(&numbered).ends_with("It also goes to a channel it names by number."));
    }

    #[derive(Default)]
    struct Fake {
        command: Option<String>,
        refuse: bool,
        writes: u32,
    }

    impl Handlers for Fake {
        fn current(&self) -> Option<String> {
            self.command.clone()
        }

        fn set(&mut self, command: &str) -> bool {
            if self.refuse {
                return false;
            }
            self.command = Some(command.to_string());
            self.writes += 1;
            true
        }

        fn clear(&mut self) -> bool {
            if self.refuse {
                return false;
            }
            self.command = None;
            self.writes += 1;
            true
        }
    }

    const HERE: &str = "D:\\Apps\\Parrotfish\\Parrotfish.exe";
    const OTHER: &str = "\"D:\\Other\\voice.exe\" \"%1\"";

    #[test]
    fn taking_the_links_and_giving_them_back_leaves_things_as_they_were() {
        let mut registry = Fake::default();
        let mut state = Claim::default();
        assert!(claim(&mut registry, HERE, &mut state));
        assert_eq!(registry.command.as_deref(), Some("\"D:\\Apps\\Parrotfish\\Parrotfish.exe\" \"%1\""));
        assert_eq!(state, Claim { on: true, command: command_for(HERE), previous: String::new() });
        assert!(claim(&mut registry, HERE, &mut state));
        assert_eq!(registry.writes, 1, "asking twice writes once");
        assert!(release(&mut registry, &mut state));
        assert_eq!(registry.command, None, "nothing was there before, so nothing is left");
        assert_eq!(state, Claim::default());

        let mut registry = Fake { command: Some(OTHER.to_string()), ..Fake::default() };
        assert!(claim(&mut registry, HERE, &mut state));
        assert_eq!(state.previous, OTHER);
        assert!(release(&mut registry, &mut state));
        assert_eq!(registry.command.as_deref(), Some(OTHER), "the program that had them gets them back");
    }

    #[test]
    fn links_another_program_took_are_left_alone() {
        let mut registry = Fake::default();
        let mut state = Claim::default();
        claim(&mut registry, HERE, &mut state);
        registry.command = Some(OTHER.to_string());
        let writes = registry.writes;
        assert!(release(&mut registry, &mut state));
        assert_eq!(registry.command.as_deref(), Some(OTHER));
        assert_eq!(registry.writes, writes);
        assert_eq!(state, Claim::default());

        claim(&mut registry, HERE, &mut state);
        registry.command = Some(OTHER.to_string());
        refresh(&mut registry, HERE, &mut state);
        assert_eq!(state, Claim::default(), "the switch goes off, Parrotfish does not take them back");
        assert_eq!(registry.command.as_deref(), Some(OTHER));
    }

    #[test]
    fn a_program_that_moved_points_the_links_at_its_new_place() {
        let mut registry = Fake::default();
        let mut state = Claim::default();
        claim(&mut registry, "D:\\Old\\Parrotfish.exe", &mut state);
        refresh(&mut registry, HERE, &mut state);
        assert_eq!(registry.command, Some(command_for(HERE)));
        assert_eq!(state.command, command_for(HERE));
        registry.command = None;
        refresh(&mut registry, HERE, &mut state);
        assert_eq!(registry.command, Some(command_for(HERE)), "a link entry that went missing is put back");
        let writes = registry.writes;
        refresh(&mut registry, &HERE.to_uppercase(), &mut state);
        assert_eq!(registry.writes, writes, "the same place in other letters is the same place");

        let mut off = Claim::default();
        let mut untouched = Fake::default();
        refresh(&mut untouched, HERE, &mut off);
        assert_eq!((untouched.command, untouched.writes, off), (None, 0, Claim::default()), "switched off means hands off");
    }

    #[test]
    fn links_taken_under_the_earlier_name_follow_the_renamed_program() {
        let mut registry = Fake { command: Some(OTHER.to_string()), ..Fake::default() };
        let mut state = Claim::default();
        claim(&mut registry, "D:\\Apps\\PhishSpeak\\PhishSpeak.exe", &mut state);
        let renamed = "D:\\Apps\\PhishSpeak\\Parrotfish.exe";
        refresh(&mut registry, renamed, &mut state);
        assert_eq!(registry.command, Some(command_for(renamed)));
        assert_eq!(state, Claim { on: true, command: command_for(renamed), previous: OTHER.to_string() });
        assert!(release(&mut registry, &mut state));
        assert_eq!(registry.command.as_deref(), Some(OTHER), "switching off still gives them back to who had them");
    }

    #[test]
    fn a_refusal_changes_nothing() {
        let mut registry = Fake { refuse: true, ..Fake::default() };
        let mut state = Claim::default();
        assert!(!claim(&mut registry, HERE, &mut state));
        assert_eq!(state, Claim::default());
        registry.refuse = false;
        claim(&mut registry, HERE, &mut state);
        registry.refuse = true;
        let before = state.clone();
        assert!(!release(&mut registry, &mut state));
        assert_eq!(state, before, "still on, so the switch can say so");
    }

    #[test]
    fn taking_the_links_again_after_moving_keeps_what_was_there_first() {
        let mut registry = Fake { command: Some(OTHER.to_string()), ..Fake::default() };
        let mut state = Claim::default();
        claim(&mut registry, "D:\\Old\\Parrotfish.exe", &mut state);
        claim(&mut registry, HERE, &mut state);
        assert_eq!(state.previous, OTHER, "Parrotfish's own old entry is not what was there before");
        release(&mut registry, &mut state);
        assert_eq!(registry.command.as_deref(), Some(OTHER));
    }
}
