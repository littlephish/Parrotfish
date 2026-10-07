use std::fs;
use std::path::PathBuf;

use ps_client::{ServerView, WhisperGroup, WhisperScope, WhisperTarget};

use crate::hotkeys::{Chord, MAX_WHISPER_KEYS, REPLY_LANE};
use crate::settings::config_dir;

pub const MAX_LIST_CHANNELS: usize = 30;
pub const MAX_LIST_PEOPLE: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Who {
    Everyone,
    Commanders,
    ServerGroup { id: u64, name: String },
    ChannelGroup { id: u64, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aim {
    List { channels: Vec<(u64, String)>, people: Vec<(String, String)> },
    Group { who: Who, scope: WhisperScope },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperKey {
    pub chord: Chord,
    pub server_uid: String,
    pub server_name: String,
    pub aim: Aim,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhisperKeys {
    pub items: Vec<WhisperKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unusable {
    OtherServer,
    NobodyThere,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Route<'a> {
    Talk,
    Whisper(&'a WhisperTarget),
    Nothing,
}

fn whisper_path() -> PathBuf {
    config_dir().join("whisper.ini")
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

fn scope_word(scope: WhisperScope) -> &'static str {
    match scope {
        WhisperScope::AllChannels => "all",
        WhisperScope::CurrentChannel => "current",
        WhisperScope::ParentChannel => "parent",
        WhisperScope::AllParentChannels => "all_parents",
        WhisperScope::ChannelFamily => "family",
        WhisperScope::WholeFamily => "whole_family",
        WhisperScope::Subchannels => "subchannels",
    }
}

fn scope_from(word: &str) -> WhisperScope {
    match word {
        "current" => WhisperScope::CurrentChannel,
        "parent" => WhisperScope::ParentChannel,
        "all_parents" => WhisperScope::AllParentChannels,
        "family" => WhisperScope::ChannelFamily,
        "whole_family" => WhisperScope::WholeFamily,
        "subchannels" => WhisperScope::Subchannels,
        _ => WhisperScope::AllChannels,
    }
}

pub fn scope_label(scope: WhisperScope) -> &'static str {
    match scope {
        WhisperScope::AllChannels => "everywhere",
        WhisperScope::CurrentChannel => "my channel",
        WhisperScope::ParentChannel => "the channel above mine",
        WhisperScope::AllParentChannels => "every channel above mine",
        WhisperScope::ChannelFamily => "my channel and all below it",
        WhisperScope::WholeFamily => "my whole branch",
        WhisperScope::Subchannels => "the channels right below mine",
    }
}

pub fn describe(aim: &Aim) -> String {
    match aim {
        Aim::List { channels, people } => {
            let names: Vec<&str> =
                channels.iter().map(|(_, name)| name.as_str()).chain(people.iter().map(|(_, name)| name.as_str())).collect();
            match names.as_slice() {
                [] => "Nobody yet".to_string(),
                [one] => one.to_string(),
                [one, two] => format!("{one} and {two}"),
                [one, rest @ ..] => format!("{one} and {} more", rest.len()),
            }
        }
        Aim::Group { who, scope } => {
            let who = match who {
                Who::Everyone => "Everyone",
                Who::Commanders => "Channel commanders",
                Who::ServerGroup { name, .. } | Who::ChannelGroup { name, .. } => name.as_str(),
            };
            format!("{who}, {}", scope_label(*scope))
        }
    }
}

pub fn phrase(aim: &Aim) -> String {
    match aim {
        Aim::Group { who: Who::Everyone, scope } => format!("everyone, {}", scope_label(*scope)),
        Aim::Group { who: Who::Commanders, scope } => format!("channel commanders, {}", scope_label(*scope)),
        other => describe(other),
    }
}

pub fn resolve(key: &WhisperKey, view: &ServerView) -> Result<WhisperTarget, Unusable> {
    let tied = !matches!(&key.aim, Aim::Group { who: Who::Everyone | Who::Commanders, .. });
    let elsewhere = !key.server_uid.is_empty() && key.server_uid != view.server.uid;
    if elsewhere || (tied && key.server_uid.is_empty()) {
        return Err(Unusable::OtherServer);
    }
    match &key.aim {
        Aim::List { channels, people } => {
            let channels: Vec<u64> = channels
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| view.channels.iter().any(|node| node.channel.id == *id))
                .take(MAX_LIST_CHANNELS)
                .collect();
            let mut clients: Vec<u16> = Vec::new();
            for node in &view.channels {
                for client in &node.clients {
                    let listed = people.iter().any(|(uid, _)| !uid.is_empty() && *uid == client.uid);
                    let room = clients.len() < MAX_LIST_PEOPLE;
                    if listed && room && client.id != view.own_id && !clients.contains(&client.id) {
                        clients.push(client.id);
                    }
                }
            }
            if channels.is_empty() && clients.is_empty() {
                return Err(Unusable::NobodyThere);
            }
            Ok(WhisperTarget::List { channels, clients })
        }
        Aim::Group { who, scope } => {
            let who = match who {
                Who::Everyone => WhisperGroup::Everyone,
                Who::Commanders => WhisperGroup::Commanders,
                Who::ServerGroup { id, .. } => WhisperGroup::ServerGroup(*id),
                Who::ChannelGroup { id, .. } => WhisperGroup::ChannelGroup(*id),
            };
            Ok(WhisperTarget::Group { who, scope: *scope })
        }
    }
}

pub fn route(lane: u8, table: &[Option<WhisperTarget>]) -> Route<'_> {
    if lane == 0 {
        return Route::Talk;
    }
    match table.get(usize::from(lane)) {
        Some(Some(target)) => Route::Whisper(target),
        _ => Route::Nothing,
    }
}

pub fn lane_table(keys: &[WhisperKey], view: Option<&ServerView>, reply_to: Option<u16>) -> Vec<Option<WhisperTarget>> {
    let mut table: Vec<Option<WhisperTarget>> = vec![None; usize::from(REPLY_LANE) + 1];
    if let Some(view) = view {
        for (index, key) in keys.iter().enumerate().take(MAX_WHISPER_KEYS) {
            table[index + 1] = resolve(key, view).ok();
        }
        table[usize::from(REPLY_LANE)] =
            reply_to.filter(|id| view.client(*id).is_some()).map(|id| WhisperTarget::List { channels: Vec::new(), clients: vec![id] });
    }
    table
}

#[derive(Default)]
struct Draft {
    chord: Chord,
    kind: String,
    scope: String,
    server_uid: String,
    server_name: String,
    group: Option<u64>,
    group_name: String,
    channels: Vec<(u64, String)>,
    people: Vec<(String, String)>,
}

impl Draft {
    fn finish(self) -> Option<WhisperKey> {
        let scope = scope_from(&self.scope);
        let aim = match self.kind.as_str() {
            "list" => Aim::List { channels: self.channels, people: self.people },
            "everyone" => Aim::Group { who: Who::Everyone, scope },
            "commanders" => Aim::Group { who: Who::Commanders, scope },
            "server_group" => Aim::Group { who: Who::ServerGroup { id: self.group?, name: self.group_name }, scope },
            "channel_group" => Aim::Group { who: Who::ChannelGroup { id: self.group?, name: self.group_name }, scope },
            _ => return None,
        };
        Some(WhisperKey { chord: self.chord, server_uid: self.server_uid, server_name: self.server_name, aim })
    }
}

impl WhisperKeys {
    pub fn parse(text: &str) -> Self {
        let mut items: Vec<WhisperKey> = Vec::new();
        let mut current: Option<Draft> = None;
        let close = |draft: Option<Draft>, items: &mut Vec<WhisperKey>| {
            if let Some(key) = draft.and_then(Draft::finish) {
                if items.len() < MAX_WHISPER_KEYS {
                    items.push(key);
                }
            }
        };
        for raw in text.lines() {
            let line = raw.trim();
            if line.eq_ignore_ascii_case("[whisper]") {
                close(current.take(), &mut items);
                current = Some(Draft::default());
                continue;
            }
            let Some(draft) = current.as_mut() else {
                continue;
            };
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match name.trim() {
                "key" => draft.chord = Chord::parse(value),
                "kind" => draft.kind = value.to_string(),
                "scope" => draft.scope = value.to_string(),
                "server" => draft.server_uid = value.to_string(),
                "server_name" => draft.server_name = value.to_string(),
                "group" => draft.group = value.parse().ok(),
                "group_name" => draft.group_name = value.to_string(),
                "channel" => {
                    let (id, label) = value.split_once(' ').unwrap_or((value, ""));
                    if let (Ok(id), true) = (id.parse::<u64>(), draft.channels.len() < MAX_LIST_CHANNELS) {
                        draft.channels.push((id, label.trim().to_string()));
                    }
                }
                "person" => {
                    let (uid, label) = value.split_once(' ').unwrap_or((value, ""));
                    if !uid.is_empty() && draft.people.len() < MAX_LIST_PEOPLE {
                        draft.people.push((uid.to_string(), label.trim().to_string()));
                    }
                }
                _ => {}
            }
        }
        close(current.take(), &mut items);
        Self { items }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for key in &self.items {
            out.push_str("[whisper]\n");
            out.push_str(&format!("key={}\n", key.chord.to_text()));
            match &key.aim {
                Aim::List { channels, people } => {
                    out.push_str("kind=list\n");
                    for (id, name) in channels {
                        out.push_str(&format!("channel={id} {}\n", one_line(name)));
                    }
                    for (uid, name) in people {
                        out.push_str(&format!("person={} {}\n", one_line(uid), one_line(name)));
                    }
                }
                Aim::Group { who, scope } => {
                    let (kind, group) = match who {
                        Who::Everyone => ("everyone", None),
                        Who::Commanders => ("commanders", None),
                        Who::ServerGroup { id, name } => ("server_group", Some((id, name))),
                        Who::ChannelGroup { id, name } => ("channel_group", Some((id, name))),
                    };
                    out.push_str(&format!("kind={kind}\nscope={}\n", scope_word(*scope)));
                    if let Some((id, name)) = group {
                        out.push_str(&format!("group={id}\ngroup_name={}\n", one_line(name)));
                    }
                }
            }
            out.push_str(&format!("server={}\nserver_name={}\n\n", one_line(&key.server_uid), one_line(&key.server_name)));
        }
        out
    }

    pub fn load() -> Self {
        fs::read_to_string(whisper_path()).map(|text| Self::parse(&text)).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        fs::create_dir_all(config_dir())?;
        fs::write(whisper_path(), self.serialize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ps_client::{Channel, ChannelNode, ClientInfo, ServerInfo};

    fn view(server_uid: &str) -> ServerView {
        let person = |id: u16, channel: u64, name: &str, uid: &str| ClientInfo {
            id,
            channel,
            nickname: name.into(),
            uid: uid.into(),
            ..ClientInfo::default()
        };
        let node = |id: u64, name: &str, clients: Vec<ClientInfo>| ChannelNode {
            channel: Channel { id, name: name.into(), ..Channel::default() },
            depth: 0,
            clients,
        };
        ServerView {
            server: ServerInfo { name: "Reef Runners".into(), uid: server_uid.into(), ..ServerInfo::default() },
            own_id: 7,
            own_channel: 1,
            channels: vec![
                node(1, "Lobby", vec![person(7, 1, "Minnow", "uidMe"), person(8, 1, "Marlin", "uidMarlin")]),
                node(2, "Deep Rock", vec![person(9, 2, "Coralline", "uidCora"), person(10, 2, "Coralline", "uidCora")]),
            ],
        }
    }

    fn list_key(server: &str) -> WhisperKey {
        WhisperKey {
            chord: Chord::new(&[0x65]),
            server_uid: server.into(),
            server_name: "Reef Runners".into(),
            aim: Aim::List {
                channels: vec![(2, "Deep Rock".into()), (40, "Gone".into())],
                people: vec![
                    ("uidCora".into(), "Coralline".into()),
                    ("uidOffline".into(), "Pike".into()),
                    ("uidMe".into(), "Minnow".into()),
                ],
            },
        }
    }

    fn group_key(server: &str, who: Who, scope: WhisperScope) -> WhisperKey {
        WhisperKey { chord: Chord::new(&[0x64]), server_uid: server.into(), server_name: String::new(), aim: Aim::Group { who, scope } }
    }

    #[test]
    fn targets_follow_the_server() {
        assert_eq!(
            resolve(&list_key("serverA"), &view("serverA")),
            Ok(WhisperTarget::List { channels: vec![2], clients: vec![9, 10] })
        );
        assert_eq!(resolve(&list_key("serverA"), &view("serverB")), Err(Unusable::OtherServer));
        let mut gone = list_key("serverA");
        gone.aim = Aim::List { channels: vec![(40, "Gone".into())], people: vec![("uidOffline".into(), "Pike".into())] };
        assert_eq!(resolve(&gone, &view("serverA")), Err(Unusable::NobodyThere));

        let everyone = group_key("", Who::Everyone, WhisperScope::ParentChannel);
        let expected = WhisperTarget::Group { who: WhisperGroup::Everyone, scope: WhisperScope::ParentChannel };
        assert_eq!(resolve(&everyone, &view("serverA")), Ok(expected.clone()));
        assert_eq!(resolve(&everyone, &view("serverB")), Ok(expected));

        let admins = group_key("serverA", Who::ServerGroup { id: 6, name: "Server Admin".into() }, WhisperScope::AllChannels);
        assert_eq!(
            resolve(&admins, &view("serverA")),
            Ok(WhisperTarget::Group { who: WhisperGroup::ServerGroup(6), scope: WhisperScope::AllChannels })
        );
        assert_eq!(resolve(&admins, &view("serverB")), Err(Unusable::OtherServer));

        let untied = group_key("", Who::ServerGroup { id: 6, name: "Server Admin".into() }, WhisperScope::AllChannels);
        assert_eq!(resolve(&untied, &view("serverA")), Err(Unusable::OtherServer));
        assert_eq!(resolve(&list_key(""), &view("serverA")), Err(Unusable::OtherServer));
        assert_eq!(resolve(&list_key(""), &view("")), Err(Unusable::OtherServer));
    }

    #[test]
    fn a_lane_never_falls_back_to_talk() {
        let keys = vec![
            list_key("serverA"),
            list_key("serverB"),
            group_key("", Who::Commanders, WhisperScope::WholeFamily),
        ];
        let table = lane_table(&keys, Some(&view("serverA")), Some(8));
        assert_eq!(table.len(), 14);
        assert_eq!(route(0, &table), Route::Talk);
        assert!(matches!(route(1, &table), Route::Whisper(WhisperTarget::List { .. })));
        assert_eq!(route(2, &table), Route::Nothing);
        assert!(matches!(route(3, &table), Route::Whisper(WhisperTarget::Group { .. })));
        assert_eq!(route(4, &table), Route::Nothing);
        assert_eq!(route(12, &table), Route::Nothing);
        assert_eq!(route(13, &table), Route::Whisper(&WhisperTarget::List { channels: vec![], clients: vec![8] }));
        assert_eq!(route(14, &table), Route::Nothing);
        assert_eq!(route(255, &table), Route::Nothing);

        let nowhere = lane_table(&keys, None, None);
        assert!((1u8..=20).all(|lane| route(lane, &nowhere) == Route::Nothing));
        assert_eq!(route(0, &nowhere), Route::Talk);
        assert_eq!(route(3, &[]), Route::Nothing);
    }

    #[test]
    fn keys_read_as_plain_words() {
        let names = |n: usize| -> Vec<(u64, String)> {
            ["Lobby", "Deep Rock", "Tide Pool"].iter().take(n).enumerate().map(|(i, name)| (i as u64 + 1, name.to_string())).collect()
        };
        assert_eq!(describe(&Aim::List { channels: names(1), people: vec![] }), "Lobby");
        assert_eq!(describe(&Aim::List { channels: names(2), people: vec![] }), "Lobby and Deep Rock");
        assert_eq!(
            describe(&Aim::List { channels: names(2), people: vec![("u".into(), "Marlin".into()), ("v".into(), "Pike".into())] }),
            "Lobby and 3 more"
        );
        assert_eq!(describe(&Aim::List { channels: vec![], people: vec![] }), "Nobody yet");
        assert_eq!(describe(&Aim::Group { who: Who::Everyone, scope: WhisperScope::ParentChannel }), "Everyone, the channel above mine");
        assert_eq!(describe(&Aim::Group { who: Who::Commanders, scope: WhisperScope::WholeFamily }), "Channel commanders, my whole branch");
        assert_eq!(
            describe(&Aim::Group { who: Who::ServerGroup { id: 6, name: "Server Admin".into() }, scope: WhisperScope::AllChannels }),
            "Server Admin, everywhere"
        );
        assert_eq!(scope_label(WhisperScope::CurrentChannel), "my channel");
        assert_eq!(scope_label(WhisperScope::AllParentChannels), "every channel above mine");
        assert_eq!(scope_label(WhisperScope::ChannelFamily), "my channel and all below it");
        assert_eq!(scope_label(WhisperScope::Subchannels), "the channels right below mine");
        assert_eq!(
            phrase(&Aim::Group { who: Who::Everyone, scope: WhisperScope::ParentChannel }),
            "everyone, the channel above mine"
        );
        assert_eq!(
            phrase(&Aim::Group { who: Who::Commanders, scope: WhisperScope::AllChannels }),
            "channel commanders, everywhere"
        );
        assert_eq!(
            phrase(&Aim::Group { who: Who::ServerGroup { id: 6, name: "Server Admin".into() }, scope: WhisperScope::AllChannels }),
            "Server Admin, everywhere"
        );
        assert_eq!(phrase(&Aim::List { channels: names(2), people: vec![] }), "Lobby and Deep Rock");
    }

    #[test]
    fn the_file_round_trips_and_survives_damage() {
        let keys = WhisperKeys {
            items: vec![
                list_key("serverA"),
                group_key("", Who::Commanders, WhisperScope::WholeFamily),
                group_key("serverA", Who::ChannelGroup { id: 5, name: "Channel = Admin ☺".into() }, WhisperScope::Subchannels),
                WhisperKey { chord: Chord::default(), ..group_key("", Who::Everyone, WhisperScope::AllChannels) },
            ],
        };
        assert_eq!(WhisperKeys::parse(&keys.serialize()), keys);

        let damaged = "junk\nkey=5\n[whisper]\nkey=101\nkind=nonsense\n[whisper]\n\n[WHISPER]\nkey=100\nkind=everyone\nscope=sideways\n[whisper]\nkey=abc\nkind=list\nserver=serverA\nchannel=notanumber Lobby\nchannel=2 Deep Rock\nperson=uidCora\nperson= \n";
        let parsed = WhisperKeys::parse(damaged);
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].aim, Aim::Group { who: Who::Everyone, scope: WhisperScope::AllChannels });
        assert_eq!(
            parsed.items[1].aim,
            Aim::List { channels: vec![(2, "Deep Rock".into())], people: vec![("uidCora".into(), String::new())] }
        );
        assert!(parsed.items[1].chord.is_empty());
        assert!(WhisperKeys::parse("").items.is_empty());
    }

    #[test]
    fn lists_and_keys_are_capped() {
        let mut text = String::from("[whisper]\nkey=101\nkind=list\nserver=s\n");
        for n in 0..50 {
            text.push_str(&format!("channel={n} C{n}\n"));
        }
        for n in 0..90 {
            text.push_str(&format!("person=uid{n} P{n}\n"));
        }
        for _ in 0..20 {
            text.push_str("[whisper]\nkey=102\nkind=commanders\nscope=all\n");
        }
        let parsed = WhisperKeys::parse(&text);
        assert_eq!(parsed.items.len(), crate::hotkeys::MAX_WHISPER_KEYS);
        match &parsed.items[0].aim {
            Aim::List { channels, people } => assert_eq!((channels.len(), people.len()), (MAX_LIST_CHANNELS, MAX_LIST_PEOPLE)),
            other => panic!("expected a list, got {other:?}"),
        }

        let mut crowd = view("s");
        crowd.channels[1].clients = (0..200u16)
            .map(|n| ClientInfo { id: 100 + n, channel: 2, uid: "uidTwin".into(), ..ClientInfo::default() })
            .collect();
        for n in 0..60u64 {
            crowd.channels.push(ChannelNode {
                channel: Channel { id: 500 + n, ..Channel::default() },
                depth: 0,
                clients: vec![],
            });
        }
        let wide = WhisperKey {
            chord: Chord::default(),
            server_uid: "s".into(),
            server_name: String::new(),
            aim: Aim::List {
                channels: (0..60u64).map(|n| (500 + n, String::new())).collect(),
                people: vec![("uidTwin".into(), String::new())],
            },
        };
        let target = resolve(&wide, &crowd).unwrap();
        match &target {
            WhisperTarget::List { channels, clients } => {
                assert_eq!((channels.len(), clients.len()), (MAX_LIST_CHANNELS, MAX_LIST_PEOPLE))
            }
            other => panic!("expected a list, got {other:?}"),
        }
        assert!(target.frame_room() >= 122);
    }
}
