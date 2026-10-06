use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use ps_client::spacer::{parse_spacer, Spacer, SpacerAlign, SpacerLine};
use ps_client::{
    ClientHandle, ConnectionState, Event, Group, ERROR_NO_WHISPER_TARGETS, ServerView, TextTarget, CODEC_OPUS_MUSIC, CODEC_OPUS_VOICE,
};

use crate::platform;

pub const MAX_CHAT_LINES: usize = 400;
const MAX_LINE_CHARS: usize = 2000;
const REPEAT_FILL_CHARS: usize = 240;
const DEFAULT_CODEC_QUALITY: u8 = 6;
const WHISPER_LINE_GAP: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowKind {
    SpacerText,
    SpacerLine,
    Gap,
    #[default]
    Channel,
    Person,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelIcon {
    #[default]
    Speaker,
    Lock,
    Music,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RowData {
    pub kind: RowKind,
    pub id: u64,
    pub depth: u32,
    pub text: String,
    pub align: SpacerAlign,
    pub line: SpacerLine,
    pub icon: ChannelIcon,
    pub count: usize,
    pub current: bool,
    pub talking: bool,
    pub me: bool,
    pub mic_muted: bool,
    pub sound_muted: bool,
    pub away: bool,
    pub tag: String,
    pub whispering: bool,
    pub commander: bool,
}

fn fill_width(pattern: &str) -> String {
    let length = pattern.chars().count().max(1);
    pattern.repeat(REPEAT_FILL_CHARS.div_ceil(length))
}

pub fn build_rows(view: &ServerView, own_talking: bool) -> Vec<RowData> {
    let mut rows = Vec::new();
    for node in &view.channels {
        let channel = &node.channel;
        let base = RowData { id: channel.id, depth: node.depth, ..RowData::default() };
        match parse_spacer(&channel.name, channel.parent) {
            Some(Spacer::Text { align, text }) => rows.push(RowData {
                kind: RowKind::SpacerText,
                text: if align == SpacerAlign::Repeat { fill_width(&text) } else { text },
                align,
                ..base
            }),
            Some(Spacer::Line(line)) => rows.push(RowData { kind: RowKind::SpacerLine, line, ..base }),
            Some(Spacer::Gap) => rows.push(RowData { kind: RowKind::Gap, ..base }),
            None => rows.push(RowData {
                kind: RowKind::Channel,
                text: channel.name.clone(),
                icon: if channel.has_password {
                    ChannelIcon::Lock
                } else if channel.codec == CODEC_OPUS_MUSIC {
                    ChannelIcon::Music
                } else {
                    ChannelIcon::Speaker
                },
                count: node.clients.len(),
                current: channel.id == view.own_channel,
                ..base
            }),
        }
        for client in &node.clients {
            let me = client.id == view.own_id;
            rows.push(RowData {
                kind: RowKind::Person,
                id: u64::from(client.id),
                depth: node.depth,
                text: client.nickname.clone(),
                talking: if me { own_talking } else { client.talking },
                me,
                mic_muted: client.input_muted || !client.input_hardware,
                sound_muted: client.output_muted || !client.output_hardware,
                away: client.away,
                whispering: client.whispering,
                commander: client.is_channel_commander,
                tag: if client.whispering {
                    "whispers to you".to_string()
                } else if client.is_query {
                    "query".to_string()
                } else if client.away {
                    "away".to_string()
                } else {
                    String::new()
                },
                ..RowData::default()
            });
        }
    }
    rows
}

pub fn next_view(sessions: &[(u16, bool)], viewed: Option<u16>, closed: u16) -> Option<u16> {
    if let Some(current) = viewed {
        if current != closed && sessions.iter().any(|(id, _)| *id == current) {
            return viewed;
        }
    }
    let position = sessions.iter().position(|(id, _)| *id == closed);
    let split = position.map(|p| p + 1).unwrap_or(0);
    let rest: Vec<(u16, bool)> = sessions[split..]
        .iter()
        .chain(sessions[..position.unwrap_or(0)].iter())
        .copied()
        .filter(|(id, _)| *id != closed)
        .collect();
    rest.iter().find(|(_, connected)| *connected).or(rest.first()).map(|(id, _)| *id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicMove {
    None,
    Switch { end_talk_on: Option<u16>, send_to: Option<u16> },
}

pub fn mic_move(old: Option<u16>, new: Option<u16>, transmitting: bool) -> MicMove {
    if old == new {
        return MicMove::None;
    }
    MicMove::Switch { end_talk_on: if transmitting { old } else { None }, send_to: new }
}

pub fn people_online(count: usize) -> String {
    if count == 1 {
        "1 person online".to_string()
    } else {
        format!("{count} people online")
    }
}

const BBCODE_TAGS: &[&str] = &[
    "b", "i", "u", "s", "url", "img", "color", "size", "center", "left", "right", "list", "*", "hr", "table", "tr",
    "td", "th",
];

pub fn strip_bbcode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let inner = after[..close].trim_start_matches('/');
        let name = inner.split('=').next().unwrap_or("").trim().to_ascii_lowercase();
        if BBCODE_TAGS.contains(&name.as_str()) {
            rest = &after[close + 1..];
        } else {
            out.push('[');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn tidy(text: &str) -> String {
    let cleaned = strip_bbcode(text).replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = cleaned.trim();
    if trimmed.chars().count() > MAX_LINE_CHARS {
        let mut cut: String = trimmed.chars().take(MAX_LINE_CHARS).collect();
        cut.push('…');
        cut
    } else {
        trimmed.to_string()
    }
}

pub fn server_error_text(id: u32, message: &str, extra: &str) -> String {
    match id {
        0x030d => "Wrong channel password.".to_string(),
        0x0309 => "That channel is full.".to_string(),
        0x0302 => "You are already in that channel.".to_string(),
        0x0a08 => "You do not have permission to do that here.".to_string(),
        0x020c | 0x0d03 => "The server says you are sending too much too quickly. Wait a moment.".to_string(),
        0x0201 => "That nickname is already in use.".to_string(),
        _ => {
            let detail = if extra.trim().is_empty() { String::new() } else { format!(" ({})", extra.trim()) };
            format!("The server answered: {}{detail}.", message.trim())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogField {
    Address,
    Nickname,
    Password,
    Identity,
}

impl DialogField {
    pub fn index(self) -> i32 {
        match self {
            DialogField::Address => 0,
            DialogField::Nickname => 1,
            DialogField::Password => 2,
            DialogField::Identity => 3,
        }
    }
}

pub fn connect_failure(reason: &str, address: &str, had_password: bool) -> (DialogField, String) {
    let lower = reason.to_lowercase();
    let has = |needle: &str| lower.contains(needle);
    if has("password") {
        let text = if had_password {
            "The server did not accept that password. Check it and try again."
        } else {
            "This server needs a password. Enter it and connect again."
        };
        return (DialogField::Password, text.to_string());
    }
    if has("no server address") {
        return (DialogField::Address, "Enter the server address.".to_string());
    }
    if has("cannot resolve") || has("has no address") {
        return (DialogField::Address, format!("Could not find {address}. Check the spelling."));
    }
    if has("no response") || has("nothing is listening") || has("cannot reach") {
        return (DialogField::Address, format!("No answer from {address}. Check the address and port."));
    }
    if has("timed out") {
        return (
            DialogField::Address,
            format!("{address} answered, but the connection did not finish. Try again in a moment."),
        );
    }
    if has("nickname") {
        return (DialogField::Nickname, "That nickname is already in use there. Choose another.".to_string());
    }
    if has("banned") {
        return (DialogField::Address, "You are banned from this server.".to_string());
    }
    if has("flood") {
        return (
            DialogField::Address,
            "The server is turning you away after too many attempts. Wait a little and try again.".to_string(),
        );
    }
    if has("maxclient") {
        return (DialogField::Address, "The server is full. Try again later.".to_string());
    }
    if has("too many clones") {
        return (DialogField::Identity, "This identity is already connected there too many times.".to_string());
    }
    if has("private key") {
        return (
            DialogField::Identity,
            "This identity has no private key, so it cannot connect. Choose another.".to_string(),
        );
    }
    if has("security level") {
        return (
            DialogField::Identity,
            "The server wants a stronger identity, and this one could not be made strong enough.".to_string(),
        );
    }
    if has("pre-3.1") {
        return (DialogField::Address, "This server is too old for PhishSpeak to talk to.".to_string());
    }
    (DialogField::Address, format!("Could not connect: {}.", reason.trim().trim_end_matches('.')))
}

fn describe_leave(nickname: &str, reason: &str) -> String {
    if reason.starts_with("kicked") || reason.starts_with("banned") {
        format!("{nickname} was {reason}")
    } else if let Some(rest) = reason.strip_prefix("connection lost") {
        format!("{nickname} lost connection{rest}")
    } else if reason.starts_with("left view") {
        format!("{nickname} went out of view")
    } else {
        format!("{nickname} {reason}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    System,
    Message,
    Mine,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub time: String,
    pub kind: ChatKind,
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    Connected,
    Closed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectRequest {
    pub name: String,
    pub address: String,
    pub nickname: String,
    pub password: String,
    pub identity_uid: String,
    pub channel: String,
    pub save_bookmark: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub tree: bool,
    pub chat: bool,
    pub header: bool,
    pub channel: bool,
    pub connected: bool,
    pub closed: Option<String>,
    pub level: Option<u8>,
    pub talking: Option<(u16, bool)>,
    pub forget: Vec<u16>,
    pub forget_all: bool,
    pub notice: Option<String>,
    pub whisper_unheard: bool,
    pub whisper_from: Option<u16>,
}

pub struct Session {
    pub id: u16,
    pub name: String,
    pub request: ConnectRequest,
    pub client: Option<ClientHandle>,
    pub events: Option<Receiver<Event>>,
    pub view: Option<ServerView>,
    pub chat: VecDeque<ChatLine>,
    pub chat_total: u64,
    pub phase: Phase,
    pub state_text: String,
    pub ping: String,
    pub talkers: usize,
    pub own_id: u16,
    pub leaving: bool,
    pub waiting_level: Option<u8>,
    pub trace: bool,
    pub whispered: HashMap<u16, Instant>,
    pub server_groups: Vec<Group>,
    pub channel_groups: Vec<Group>,
}

impl Session {
    pub fn new(id: u16, request: ConnectRequest, trace: bool) -> Self {
        let name = if request.name.trim().is_empty() { request.address.clone() } else { request.name.clone() };
        Self {
            id,
            name,
            request,
            client: None,
            events: None,
            view: None,
            chat: VecDeque::new(),
            chat_total: 0,
            phase: Phase::Connecting,
            state_text: "Connecting".to_string(),
            ping: String::new(),
            talkers: 0,
            own_id: 0,
            leaving: false,
            waiting_level: None,
            trace,
            whispered: HashMap::new(),
            server_groups: Vec::new(),
            channel_groups: Vec::new(),
        }
    }

    pub fn attach(&mut self, client: ClientHandle, events: Receiver<Event>) {
        self.client = Some(client);
        self.events = Some(events);
        self.phase = Phase::Connecting;
        self.state_text = "Connecting".to_string();
    }

    pub fn is_connected(&self) -> bool {
        self.phase == Phase::Connected
    }

    pub fn drain(&mut self) -> Vec<Event> {
        match &self.events {
            Some(events) => events.try_iter().collect(),
            None => Vec::new(),
        }
    }

    pub fn push_line(&mut self, kind: ChatKind, name: &str, text: &str) {
        let text = tidy(text);
        if text.is_empty() {
            return;
        }
        let stamp = platform::timestamp();
        self.chat.push_back(ChatLine {
            time: stamp.chars().take(5).collect(),
            kind,
            name: name.trim().to_string(),
            text,
        });
        self.chat_total += 1;
        while self.chat.len() > MAX_CHAT_LINES {
            self.chat.pop_front();
        }
    }

    fn system(&mut self, text: &str) {
        self.push_line(ChatKind::System, "", text);
    }

    pub fn channel_name(&self, id: u64) -> String {
        self.view
            .as_ref()
            .and_then(|v| v.channels.iter().find(|n| n.channel.id == id))
            .map(|n| n.channel.name.clone())
            .unwrap_or_else(|| "another channel".to_string())
    }

    pub fn own_channel_name(&self) -> String {
        self.view
            .as_ref()
            .and_then(|v| v.own_channel_node())
            .map(|n| n.channel.name.clone())
            .unwrap_or_default()
    }

    pub fn own_codec(&self) -> (u8, u8) {
        self.view
            .as_ref()
            .and_then(|v| v.own_channel_node())
            .map(|n| (n.channel.codec, n.channel.codec_quality))
            .unwrap_or((CODEC_OPUS_VOICE, DEFAULT_CODEC_QUALITY))
    }

    pub fn own_nickname(&self) -> String {
        self.view
            .as_ref()
            .and_then(|v| v.client(v.own_id))
            .map(|c| c.nickname.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.request.nickname.clone())
    }

    pub fn detail(&self) -> String {
        match (&self.phase, &self.view) {
            (Phase::Connected, Some(view)) => people_online(view.client_count()),
            (Phase::Closed(_), _) => "Disconnected".to_string(),
            _ => self.state_text.clone(),
        }
    }

    pub fn short_detail(&self) -> String {
        match (&self.phase, &self.view) {
            (Phase::Connected, Some(view)) => format!("{} online", view.client_count()),
            (Phase::Closed(_), _) => "Disconnected".to_string(),
            _ => self.state_text.clone(),
        }
    }

    fn count_talkers(&mut self) {
        self.talkers = self
            .view
            .as_ref()
            .map(|v| v.channels.iter().flat_map(|n| n.clients.iter()).filter(|c| c.talking).count())
            .unwrap_or(0);
    }

    pub fn apply(&mut self, event: Event) -> Outcome {
        let mut out = Outcome::default();
        let lines_before = self.chat_total;
        match event {
            Event::Log(text) => {
                if self.trace || text.starts_with("Warning") {
                    self.system(&text);
                }
            }
            Event::State(state) => {
                let text = match state {
                    ConnectionState::Resolving => "Finding the server",
                    ConnectionState::Connecting => "Connecting",
                    ConnectionState::Handshake => "Securing the connection",
                    ConnectionState::Connected => "Connected",
                    ConnectionState::Disconnecting => "Disconnecting",
                    ConnectionState::Disconnected => "",
                };
                if !text.is_empty() {
                    self.state_text = text.to_string();
                    out.header = true;
                }
            }
            Event::Connected { client_id, server } => {
                self.phase = Phase::Connected;
                self.own_id = client_id;
                self.waiting_level = None;
                self.request.password.clear();
                if self.request.name.trim().is_empty() && !server.name.trim().is_empty() {
                    self.name = server.name.trim().to_string();
                }
                let shown = if server.name.trim().is_empty() { self.name.clone() } else { server.name.clone() };
                self.system(&format!("Connected to {shown}"));
                if !server.welcome_message.trim().is_empty() {
                    self.system(&server.welcome_message);
                }
                out.connected = true;
                out.header = true;
            }
            Event::View(view) => {
                if self.request.name.trim().is_empty() && !view.server.name.trim().is_empty() {
                    self.name = view.server.name.trim().to_string();
                }
                let before = self.view.as_ref().map(|v| (v.own_channel, self.own_codec()));
                self.view = Some(view);
                let after = self.view.as_ref().map(|v| (v.own_channel, self.own_codec()));
                out.channel = before != after;
                self.count_talkers();
                out.tree = true;
                out.header = true;
            }
            Event::Talking { client_id, talking, whisper } => {
                let mut name = String::new();
                if let Some(view) = &mut self.view {
                    for node in &mut view.channels {
                        for client in &mut node.clients {
                            if client.id == client_id {
                                client.talking = talking;
                                client.whispering = talking && whisper;
                                name = client.nickname.clone();
                            }
                        }
                    }
                }
                let before = self.talkers;
                self.count_talkers();
                out.talking = Some((client_id, talking));
                out.header = (before > 0) != (self.talkers > 0);
                if talking && whisper {
                    out.whisper_from = Some(client_id);
                    let now = Instant::now();
                    let recent = self.whispered.get(&client_id).is_some_and(|at| now.duration_since(*at) < WHISPER_LINE_GAP);
                    self.whispered.insert(client_id, now);
                    if !recent && !name.is_empty() {
                        self.system(&format!("{name} is whispering to you"));
                    }
                }
            }
            Event::TextMessage { target, from_id, from_name, text } => {
                let mine = self.own_id != 0 && from_id == self.own_id;
                let name = match target {
                    TextTarget::Channel => from_name,
                    TextTarget::Server => format!("{from_name} (server)"),
                    TextTarget::Client(_) => format!("{from_name} (private)"),
                };
                self.push_line(if mine { ChatKind::Mine } else { ChatKind::Message }, &name, &text);
            }
            Event::Poke { from_name, text } => {
                let line = if text.trim().is_empty() {
                    format!("{from_name} poked you")
                } else {
                    format!("{from_name} poked you: {}", tidy(&text))
                };
                self.system(&line);
                out.notice = Some(line);
            }
            Event::ClientEntered { client } => {
                if !client.is_query {
                    self.system(&format!("{} connected", client.nickname));
                }
            }
            Event::ClientLeft { client, reason } => {
                out.forget.push(client.id);
                if !client.is_query {
                    self.system(&describe_leave(&client.nickname, &reason));
                }
            }
            Event::ClientMoved { client, to, .. } => {
                let channel = self.channel_name(to);
                if client.id == self.own_id {
                    out.forget_all = true;
                    self.system(&format!("You joined {channel}"));
                } else {
                    out.forget.push(client.id);
                    self.system(&format!("{} moved to {channel}", client.nickname));
                }
            }
            Event::ServerError { id, message, extra } => {
                if id == ERROR_NO_WHISPER_TARGETS {
                    out.whisper_unheard = true;
                } else {
                    let text = server_error_text(id, &message, &extra);
                    self.push_line(ChatKind::Error, "", &text);
                }
            }
            Event::SecurityLevelRequired(level) => {
                self.waiting_level = Some(level);
                self.system(&format!(
                    "This server asks for identity security level {level}. Working on it, which can take a while."
                ));
                out.level = Some(level);
            }
            Event::Groups { server_groups, channel_groups } => {
                self.server_groups = server_groups;
                self.channel_groups = channel_groups;
            }
            Event::Stats(stats) => {
                let text = format!("{:.0} ms", stats.ping_ms.max(0.0));
                if text != self.ping {
                    self.ping = text;
                    out.header = true;
                }
            }
            Event::Disconnected { reason } => {
                self.client = None;
                self.events = None;
                self.view = None;
                self.talkers = 0;
                self.ping.clear();
                out.forget_all = true;
                out.tree = true;
                out.header = true;
                if self.waiting_level.is_some() && !self.leaving {
                    self.phase = Phase::Connecting;
                    self.state_text = "Making your identity stronger".to_string();
                } else {
                    self.phase = Phase::Closed(reason.clone());
                    out.closed = Some(reason);
                }
            }
        }
        out.chat = self.chat_total != lines_before;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ps_client::{Channel, ChannelNode, ClientInfo, LinkStats, ServerInfo};

    fn channel(id: u64, parent: u64, name: &str) -> Channel {
        Channel { id, parent, name: name.into(), codec: CODEC_OPUS_VOICE, codec_quality: 6, ..Channel::default() }
    }

    fn person(id: u16, channel: u64, nickname: &str) -> ClientInfo {
        ClientInfo {
            id,
            channel,
            nickname: nickname.into(),
            input_hardware: true,
            output_hardware: true,
            ..ClientInfo::default()
        }
    }

    fn sample_view() -> ServerView {
        let mut marlin = person(8, 1, "Marlin");
        marlin.talking = true;
        let mut coralline = person(9, 1, "Coralline");
        coralline.input_muted = true;
        coralline.away = true;
        let mut headless = person(11, 4, "Jukebox");
        headless.output_hardware = false;
        let mut query = person(12, 4, "serveradmin");
        query.is_query = true;
        let mut locked = channel(3, 0, "Squad Alpha");
        locked.has_password = true;
        let mut music = channel(2, 0, "Radio");
        music.codec = CODEC_OPUS_MUSIC;
        ServerView {
            server: ServerInfo { name: "Reef Runners".into(), ..ServerInfo::default() },
            own_id: 7,
            own_channel: 1,
            channels: vec![
                ChannelNode { channel: channel(20, 0, "[cspacer]Reef Runners"), depth: 0, clients: vec![] },
                ChannelNode {
                    channel: channel(1, 0, "Lobby"),
                    depth: 0,
                    clients: vec![coralline, person(7, 1, "Minnow"), marlin],
                },
                ChannelNode { channel: channel(21, 0, "[*spacer1]---"), depth: 0, clients: vec![] },
                ChannelNode { channel: locked, depth: 0, clients: vec![] },
                ChannelNode { channel: music, depth: 0, clients: vec![] },
                ChannelNode { channel: channel(4, 2, "Nested"), depth: 1, clients: vec![headless, query] },
                ChannelNode { channel: channel(22, 0, "[spacer2]"), depth: 0, clients: vec![] },
                ChannelNode { channel: channel(23, 0, "[*spacer3]-="), depth: 0, clients: vec![] },
                ChannelNode { channel: channel(24, 2, "[cspacer]Not a spacer"), depth: 1, clients: vec![] },
                ChannelNode { channel: channel(25, 0, "[cspacer"), depth: 0, clients: vec![] },
            ],
        }
    }

    #[test]
    fn rows_follow_the_tree() {
        let rows = build_rows(&sample_view(), true);
        let kinds: Vec<RowKind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RowKind::SpacerText,
                RowKind::Channel,
                RowKind::Person,
                RowKind::Person,
                RowKind::Person,
                RowKind::SpacerLine,
                RowKind::Channel,
                RowKind::Channel,
                RowKind::Channel,
                RowKind::Person,
                RowKind::Person,
                RowKind::Gap,
                RowKind::SpacerText,
                RowKind::Channel,
                RowKind::Channel,
            ]
        );

        assert_eq!((rows[0].text.as_str(), rows[0].align, rows[0].count), ("Reef Runners", SpacerAlign::Center, 0));
        assert_eq!((rows[1].text.as_str(), rows[1].icon, rows[1].count), ("Lobby", ChannelIcon::Speaker, 3));
        assert!(rows[1].current);

        let coralline = &rows[2];
        assert_eq!(coralline.text, "Coralline");
        assert!(coralline.mic_muted && coralline.away && !coralline.sound_muted && !coralline.me);
        assert_eq!(coralline.tag, "away");

        let me = &rows[3];
        assert!(me.me && me.talking);
        assert_eq!((me.id, me.depth), (7, 0));

        assert!(rows[4].talking && !rows[4].me);

        assert_eq!((rows[5].line, rows[5].count), (SpacerLine::Dashed, 0));
        assert_eq!((rows[6].icon, rows[6].id), (ChannelIcon::Lock, 3));
        assert_eq!((rows[7].icon, rows[7].depth), (ChannelIcon::Music, 0));
        assert_eq!((rows[8].text.as_str(), rows[8].depth, rows[8].count), ("Nested", 1, 2));
        assert!(!rows[8].current);
        assert_eq!(rows[9].depth, 1);
        assert!(rows[9].sound_muted);
        assert_eq!(rows[10].tag, "query");

        assert_eq!(rows[12].align, SpacerAlign::Repeat);
        assert!(rows[12].text.starts_with("-=-=-=") && rows[12].text.chars().count() >= REPEAT_FILL_CHARS);
        assert_eq!((rows[13].text.as_str(), rows[13].depth), ("[cspacer]Not a spacer", 1));
        assert_eq!(rows[14].text, "[cspacer");

        let silent = build_rows(&sample_view(), false);
        assert!(!silent[3].talking);
        assert!(build_rows(&ServerView::default(), true).is_empty());
    }

    #[test]
    fn next_view_after_a_close() {
        let sessions = [(1, true), (2, false), (3, true), (4, true)];
        assert_eq!(next_view(&sessions, Some(1), 1), Some(3));
        assert_eq!(next_view(&sessions, Some(3), 3), Some(4));
        assert_eq!(next_view(&sessions, Some(4), 4), Some(1));
        assert_eq!(next_view(&sessions, Some(3), 1), Some(3));
        assert_eq!(next_view(&sessions, Some(3), 2), Some(3));
        assert_eq!(next_view(&[(1, true), (2, false)], Some(1), 1), Some(2));
        assert_eq!(next_view(&[(5, true)], Some(5), 5), None);
        assert_eq!(next_view(&[], Some(5), 5), None);
        assert_eq!(next_view(&[(2, true)], Some(5), 5), Some(2));
        assert_eq!(next_view(&[(1, false), (2, true)], None, 9), Some(2));
    }

    #[test]
    fn microphone_follows_the_view() {
        assert_eq!(mic_move(Some(1), Some(1), true), MicMove::None);
        assert_eq!(mic_move(None, None, false), MicMove::None);
        assert_eq!(
            mic_move(Some(1), Some(2), true),
            MicMove::Switch { end_talk_on: Some(1), send_to: Some(2) }
        );
        assert_eq!(mic_move(Some(1), Some(2), false), MicMove::Switch { end_talk_on: None, send_to: Some(2) });
        assert_eq!(mic_move(Some(1), None, true), MicMove::Switch { end_talk_on: Some(1), send_to: None });
        assert_eq!(mic_move(None, Some(4), true), MicMove::Switch { end_talk_on: None, send_to: Some(4) });
    }

    #[test]
    fn bbcode_is_removed_but_brackets_survive() {
        assert_eq!(strip_bbcode("[b]Welcome[/b] to [URL=https://example.net]the reef[/url]"), "Welcome to the reef");
        assert_eq!(strip_bbcode("[color=#ff0000]red[/color] [1/2] done"), "red [1/2] done");
        assert_eq!(strip_bbcode("array[3] and [unclosed"), "array[3] and [unclosed");
        assert_eq!(strip_bbcode("[cspacer]Games"), "[cspacer]Games");
        assert_eq!(strip_bbcode("plain"), "plain");
        assert_eq!(strip_bbcode("[[b]]"), "[]");
        assert_eq!(strip_bbcode(""), "");
    }

    #[test]
    fn failures_point_at_the_right_field() {
        let (field, text) =
            connect_failure("no response from reef.example.net:9987 (is a TeamSpeak 3 server running there?)", "reef.example.net", false);
        assert_eq!(field, DialogField::Address);
        assert_eq!(text, "No answer from reef.example.net. Check the address and port.");

        let refused = "the server refused the connection: invalid server password";
        assert_eq!(connect_failure(refused, "a", true).0, DialogField::Password);
        assert!(connect_failure(refused, "a", true).1.contains("did not accept"));
        assert!(connect_failure(refused, "a", false).1.contains("needs a password"));

        assert_eq!(connect_failure("cannot resolve nope.invalid: no such host", "nope.invalid", false).0, DialogField::Address);
        assert!(connect_failure("cannot resolve nope.invalid: x", "nope.invalid", false).1.starts_with("Could not find nope.invalid"));
        assert_eq!(
            connect_failure("the server refused the connection: nickname is already in use", "a", false).0,
            DialogField::Nickname
        );
        assert_eq!(connect_failure("the selected identity has no private key", "a", false).0, DialogField::Identity);
        assert_eq!(
            connect_failure("the server requires identity security level 30", "a", false).0,
            DialogField::Identity
        );
        assert_eq!(
            connect_failure("the server refused the connection: connection failed, you are banned", "a", false).1,
            "You are banned from this server."
        );
        assert_eq!(connect_failure("something odd.", "a", false).1, "Could not connect: something odd.");
    }

    #[test]
    fn server_errors_read_as_sentences() {
        assert_eq!(server_error_text(0x030d, "invalid channel password", ""), "Wrong channel password.");
        assert_eq!(server_error_text(0x0309, "channel maxclient reached", ""), "That channel is full.");
        assert_eq!(
            server_error_text(0x0999, "something new", "detail"),
            "The server answered: something new (detail)."
        );
    }

    fn session() -> Session {
        Session::new(
            3,
            ConnectRequest {
                address: "reef.example.net".into(),
                nickname: "Minnow".into(),
                password: "hunter2".into(),
                ..ConnectRequest::default()
            },
            false,
        )
    }

    #[test]
    fn a_session_tracks_its_connection() {
        let mut s = session();
        assert_eq!(s.name, "reef.example.net");
        assert_eq!(s.detail(), "Connecting");

        let out = s.apply(Event::State(ConnectionState::Handshake));
        assert!(out.header && !out.chat);
        assert_eq!(s.detail(), "Securing the connection");
        assert!(!s.apply(Event::Log("Server identity abc".into())).chat);
        assert!(s.apply(Event::Log("Warning: the license expired".into())).chat);

        let out = s.apply(Event::Connected {
            client_id: 7,
            server: ServerInfo {
                name: "Reef Runners".into(),
                welcome_message: "[b]Mind the coral[/b]".into(),
                ..ServerInfo::default()
            },
        });
        assert!(out.connected && out.chat && out.header);
        assert!(s.is_connected());
        assert_eq!(s.name, "Reef Runners");
        assert_eq!(s.request.password, "");
        assert_eq!(s.chat.back().unwrap().text, "Mind the coral");

        let out = s.apply(Event::View(sample_view()));
        assert!(out.tree && out.header && out.channel);
        assert_eq!(s.detail(), "5 people online");
        assert_eq!(s.talkers, 1);
        assert_eq!(s.own_channel_name(), "Lobby");
        assert_eq!(s.own_nickname(), "Minnow");
        assert!(!s.apply(Event::View(sample_view())).channel);

        let out = s.apply(Event::Talking { client_id: 8, talking: false, whisper: false });
        assert_eq!(out.talking, Some((8, false)));
        assert!(out.header);
        assert_eq!(s.talkers, 0);

        s.apply(Event::TextMessage {
            target: TextTarget::Channel,
            from_id: 8,
            from_name: "Marlin".into(),
            text: "found it".into(),
        });
        let last = s.chat.back().unwrap();
        assert_eq!((last.kind, last.name.as_str(), last.text.as_str()), (ChatKind::Message, "Marlin", "found it"));
        assert_eq!(last.time.len(), 5);

        s.apply(Event::TextMessage {
            target: TextTarget::Server,
            from_id: 7,
            from_name: "Minnow".into(),
            text: "hello all".into(),
        });
        let last = s.chat.back().unwrap();
        assert_eq!((last.kind, last.name.as_str()), (ChatKind::Mine, "Minnow (server)"));

        let out = s.apply(Event::ClientLeft { client: person(9, 1, "Coralline"), reason: "left (bye)".into() });
        assert_eq!(out.forget, vec![9]);
        assert_eq!(s.chat.back().unwrap().text, "Coralline left (bye)");

        let out = s.apply(Event::ClientMoved { client: person(7, 1, "Minnow"), from: 1, to: 3 });
        assert!(out.forget_all);
        assert_eq!(s.chat.back().unwrap().text, "You joined Squad Alpha");

        let out = s.apply(Event::ClientMoved { client: person(8, 1, "Marlin"), from: 1, to: 2 });
        assert_eq!(out.forget, vec![8]);
        assert_eq!(s.chat.back().unwrap().text, "Marlin moved to Radio");

        s.apply(Event::ServerError { id: 0x030d, message: "invalid channel password".into(), extra: String::new() });
        assert_eq!(s.chat.back().unwrap().kind, ChatKind::Error);

        let out = s.apply(Event::Poke { from_name: "Marlin".into(), text: "wake up".into() });
        assert_eq!(out.notice.as_deref(), Some("Marlin poked you: wake up"));

        let out = s.apply(Event::Stats(LinkStats {
            ping_ms: 23.4,
            ping_deviation_ms: 1.0,
            packets_resent: 0,
            voice_packets_in: 0,
            voice_packets_out: 0,
        }));
        assert!(out.header);
        assert_eq!(s.ping, "23 ms");

        let out = s.apply(Event::Disconnected { reason: "kicked from the server by Marlin".into() });
        assert_eq!(out.closed.as_deref(), Some("kicked from the server by Marlin"));
        assert!(out.forget_all);
        assert_eq!(s.phase, Phase::Closed("kicked from the server by Marlin".into()));
        assert!(s.view.is_none());
    }

    #[test]
    fn a_session_waits_while_its_identity_is_improved() {
        let mut s = session();
        let out = s.apply(Event::SecurityLevelRequired(12));
        assert_eq!(out.level, Some(12));
        assert!(out.chat);
        let out = s.apply(Event::Disconnected { reason: "the server requires identity security level 12".into() });
        assert_eq!(out.closed, None);
        assert_eq!(s.phase, Phase::Connecting);
        assert_eq!(s.detail(), "Making your identity stronger");
        assert_eq!(s.request.password, "hunter2");

        let mut cancelled = session();
        cancelled.apply(Event::SecurityLevelRequired(12));
        cancelled.leaving = true;
        let out = cancelled.apply(Event::Disconnected { reason: "connection attempt cancelled".into() });
        assert!(out.closed.is_some());
    }

    #[test]
    fn whispers_and_unheard_whispers_are_noted() {
        let mut s = session();
        s.apply(Event::Connected { client_id: 7, server: ServerInfo { name: "Reef Runners".into(), ..ServerInfo::default() } });
        s.apply(Event::View(sample_view()));
        let lines = s.chat.len();
        let out = s.apply(Event::Talking { client_id: 8, talking: true, whisper: true });
        assert_eq!(out.whisper_from, Some(8));
        assert_eq!(s.chat.back().unwrap().text, "Marlin is whispering to you");
        assert_eq!(s.chat.len(), lines + 1);
        let rows = build_rows(s.view.as_ref().unwrap(), false);
        assert!(rows.iter().any(|row| row.text == "Marlin" && row.whispering && row.talking));
        s.apply(Event::Talking { client_id: 8, talking: false, whisper: false });
        let again = s.apply(Event::Talking { client_id: 8, talking: true, whisper: true });
        assert_eq!(again.whisper_from, Some(8));
        assert_eq!(s.chat.len(), lines + 1);
        let plain = s.apply(Event::Talking { client_id: 8, talking: true, whisper: false });
        assert_eq!(plain.whisper_from, None);

        let before = s.chat.len();
        let out = s.apply(Event::ServerError { id: 0x070c, message: "no whisper targets found".into(), extra: String::new() });
        assert!(out.whisper_unheard);
        assert_eq!(s.chat.len(), before);
    }

    #[test]
    fn chat_history_is_capped() {
        let mut s = session();
        for i in 0..MAX_CHAT_LINES + 25 {
            s.push_line(ChatKind::System, "", &format!("line {i}"));
        }
        s.push_line(ChatKind::System, "", "   ");
        assert_eq!(s.chat.len(), MAX_CHAT_LINES);
        assert_eq!(s.chat_total, (MAX_CHAT_LINES + 25) as u64);
        assert_eq!(s.chat.front().unwrap().text, "line 25");
        let long = "x".repeat(MAX_LINE_CHARS + 50);
        s.push_line(ChatKind::Message, "Marlin", &long);
        assert_eq!(s.chat.back().unwrap().text.chars().count(), MAX_LINE_CHARS + 1);
    }

    #[test]
    fn leaving_reads_naturally() {
        assert_eq!(describe_leave("Marlin", "left"), "Marlin left");
        assert_eq!(
            describe_leave("Marlin", "kicked from the server by Coralline (spam)"),
            "Marlin was kicked from the server by Coralline (spam)"
        );
        assert_eq!(describe_leave("Marlin", "connection lost"), "Marlin lost connection");
        assert_eq!(people_online(1), "1 person online");
        assert_eq!(people_online(12), "12 people online");
    }
}
