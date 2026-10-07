use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use ps_client::spacer::{parse_spacer, Spacer, SpacerAlign, SpacerLine};
use ps_client::{
    ChannelNode, ClientHandle, ConnectionState, Event, Group, ERROR_NO_WHISPER_TARGETS, ServerView, TextTarget, CODEC_OPUS_MUSIC, CODEC_OPUS_VOICE,
};

use crate::platform;
use crate::settings::MAX_REMEMBERED_FOLDS;

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
    pub foldable: bool,
    pub folded: bool,
    pub icons: Vec<u32>,
}

pub const MAX_ROW_ICONS: usize = 4;

fn fill_width(pattern: &str) -> String {
    let length = pattern.chars().count().max(1);
    pattern.repeat(REPEAT_FILL_CHARS.div_ceil(length))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FoldMode {
    #[default]
    Open,
    Empty,
    All,
}

impl FoldMode {
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => FoldMode::Open,
            2 => FoldMode::All,
            _ => FoldMode::Empty,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folds {
    pub mode: FoldMode,
    pub chosen: HashMap<u64, bool>,
}

fn branch_end(nodes: &[ChannelNode], index: usize) -> usize {
    let depth = nodes[index].depth;
    let mut end = index + 1;
    while end < nodes.len() && nodes[end].depth > depth {
        end += 1;
    }
    end
}

fn fold_state(view: &ServerView, index: usize, folds: &Folds) -> (bool, bool) {
    let nodes = &view.channels;
    let node = &nodes[index];
    let below = &nodes[index + 1..branch_end(nodes, index)];
    let spacer = parse_spacer(&node.channel.name, node.channel.parent).is_some();
    let foldable = !spacer && (!below.is_empty() || !node.clients.is_empty());
    let nobody = node.clients.is_empty() && below.iter().all(|n| n.clients.is_empty());
    let mine = node.channel.id == view.own_channel || below.iter().any(|n| n.channel.id == view.own_channel);
    let by_default = match folds.mode {
        FoldMode::Open => false,
        FoldMode::Empty => !below.is_empty() && nobody,
        FoldMode::All => !mine,
    };
    (foldable, foldable && folds.chosen.get(&node.channel.id).copied().unwrap_or(by_default))
}

#[cfg(test)]
pub fn build_rows(view: &ServerView, own_talking: bool) -> Vec<RowData> {
    build_rows_folded(view, own_talking, &Folds::default())
}

pub fn build_rows_folded(view: &ServerView, own_talking: bool, folds: &Folds) -> Vec<RowData> {
    let nodes = &view.channels;
    let mut rows = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        let node = &nodes[index];
        let channel = &node.channel;
        let end = branch_end(nodes, index);
        let (foldable, folded) = fold_state(view, index, folds);
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
            None => {
                let inside = || nodes[index..end].iter().flat_map(|n| n.clients.iter());
                let holds_me = nodes[index + 1..end].iter().any(|n| n.channel.id == view.own_channel);
                rows.push(RowData {
                    kind: RowKind::Channel,
                    text: channel.name.clone(),
                    icon: if channel.has_password {
                        ChannelIcon::Lock
                    } else if channel.codec == CODEC_OPUS_MUSIC {
                        ChannelIcon::Music
                    } else {
                        ChannelIcon::Speaker
                    },
                    count: if folded { inside().count() } else { node.clients.len() },
                    current: channel.id == view.own_channel || (folded && holds_me),
                    talking: folded && inside().any(|c| if c.id == view.own_id { own_talking } else { c.talking }),
                    foldable,
                    folded,
                    icons: if channel.icon == 0 { Vec::new() } else { vec![channel.icon] },
                    ..base
                })
            }
        }
        if folded {
            index = end;
            continue;
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
                icons: client.icons.iter().copied().take(MAX_ROW_ICONS).collect(),
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
        index += 1;
    }
    rows
}

pub fn toggle_fold(view: &ServerView, folds: &mut Folds, channel: u64) {
    let Some(index) = view.channels.iter().position(|n| n.channel.id == channel) else {
        return;
    };
    let (foldable, folded) = fold_state(view, index, folds);
    if foldable {
        folds.chosen.insert(channel, !folded);
    }
}

pub fn reveal(view: &ServerView, folds: &mut Folds, channel: u64) -> bool {
    let mut id = channel;
    let mut changed = false;
    for _ in 0..=view.channels.len() {
        if folds.chosen.get(&id) == Some(&true) {
            folds.chosen.remove(&id);
            changed = true;
        }
        match view.channels.iter().find(|n| n.channel.id == id) {
            Some(node) if node.channel.parent != 0 => id = node.channel.parent,
            _ => break,
        }
    }
    changed
}

pub fn remembered(folds: &Folds, view: Option<&ServerView>) -> BTreeMap<u64, bool> {
    let exists = |id: u64| view.map_or(true, |v| v.channels.iter().any(|node| node.channel.id == id));
    let mut kept: Vec<(u64, bool)> =
        folds.chosen.iter().map(|(id, folded)| (*id, *folded)).filter(|(id, _)| exists(*id)).collect();
    kept.sort_unstable();
    kept.truncate(MAX_REMEMBERED_FOLDS);
    kept.into_iter().collect()
}

fn hush_whispers(view: &mut ServerView) {
    for client in view.channels.iter_mut().flat_map(|node| node.clients.iter_mut()) {
        if client.whispering {
            client.talking = false;
            client.whispering = false;
        }
    }
}

pub fn channel_path(view: &ServerView, id: u64) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    let mut current = id;
    for _ in 0..=view.channels.len() {
        let node = view.channels.iter().find(|n| n.channel.id == current)?;
        names.push(node.channel.name.replace('/', "\\/"));
        if node.channel.parent == 0 {
            names.reverse();
            return Some(names.join("/"));
        }
        current = node.channel.parent;
    }
    None
}

pub fn find_start_channel(view: &ServerView, path: &str, id: u64) -> Option<u64> {
    let by_path = if path.trim().is_empty() {
        None
    } else {
        view.channels.iter().map(|n| n.channel.id).find(|id| channel_path(view, *id).as_deref() == Some(path))
    };
    by_path.or_else(|| view.channels.iter().map(|n| n.channel.id).find(|known| id != 0 && *known == id))
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
    pub channel_id: u64,
    pub channel_password: String,
    pub save_bookmark: bool,
    pub quiet: bool,
}

pub fn should_retry(reason: &str) -> bool {
    const PASSING: [&str; 8] = [
        "connection lost",
        "server stopped",
        "server is shutting down",
        "no response",
        "nothing is listening",
        "the handshake with the server timed out",
        "cannot resolve",
        "cannot find the address",
    ];
    PASSING.iter().any(|start| reason.starts_with(start))
}

fn sentence(text: &str) -> String {
    let text = text.trim().trim_end_matches('.');
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(match attempt {
        0 | 1 => 2,
        2 => 4,
        3 => 8,
        4 => 15,
        _ => 30,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub id: u16,
    pub uid: String,
    pub name: String,
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
    pub groups: bool,
    pub folds: bool,
    pub start_channel: Option<(u64, bool)>,
    pub start_password: String,
    pub retry: bool,
    pub icon: Option<(u32, Result<Vec<u8>, String>)>,
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
    pub folds: Folds,
    pub allow_whispers: bool,
    pub reply_to: Option<(u16, String)>,
    pub server_uid: String,
    pub start_pending: bool,
    pub was_connected: bool,
    pub retries: u32,
    pub retry_at: Option<Instant>,
    pub kept_password: String,
    pub last_channel: (String, u64),
    pub peer: Option<Peer>,
    pub voices_applied: HashMap<u16, (String, f32)>,
    pub silenced: HashSet<u16>,
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
            folds: Folds::default(),
            allow_whispers: true,
            reply_to: None,
            server_uid: String::new(),
            start_pending: false,
            was_connected: false,
            retries: 0,
            retry_at: None,
            kept_password: String::new(),
            last_channel: (String::new(), 0),
            peer: None,
            voices_applied: HashMap::new(),
            silenced: HashSet::new(),
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

    pub fn set_allow_whispers(&mut self, allow: bool) {
        self.allow_whispers = allow;
        if !allow {
            if let Some(view) = &mut self.view {
                hush_whispers(view);
            }
            self.count_talkers();
        }
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

    pub fn system(&mut self, text: &str) {
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
                self.server_uid = server.uid.clone();
                self.start_pending = !self.request.channel.trim().is_empty() || self.request.channel_id != 0;
                self.waiting_level = None;
                self.was_connected = true;
                self.retries = 0;
                self.retry_at = None;
                self.kept_password = std::mem::take(&mut self.request.password);
                if !self.start_pending {
                    self.request.channel_password.clear();
                }
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
            Event::View(mut view) => {
                if !self.allow_whispers {
                    hush_whispers(&mut view);
                }
                if self.request.name.trim().is_empty() && !view.server.name.trim().is_empty() {
                    self.name = view.server.name.trim().to_string();
                }
                let before = self.view.as_ref().map(|v| (v.own_channel, self.own_codec()));
                if view.own_channel != 0 {
                    if let Some(path) = channel_path(&view, view.own_channel) {
                        self.last_channel = (path, view.own_channel);
                    }
                }
                self.view = Some(view);
                let after = self.view.as_ref().map(|v| (v.own_channel, self.own_codec()));
                out.channel = before != after;
                if let Some(view) = &self.view {
                    if before.map(|(channel, _)| channel) != Some(view.own_channel) {
                        out.folds = reveal(view, &mut self.folds, view.own_channel);
                    }
                }
                if let (true, Some(view)) = (self.start_pending, &self.view) {
                    if view.own_channel != 0 {
                        self.start_pending = false;
                        let password = std::mem::take(&mut self.request.channel_password);
                        let wanted = find_start_channel(view, &self.request.channel, self.request.channel_id);
                        out.start_channel = wanted.filter(|id| *id != view.own_channel).map(|id| {
                            let locked = view.channels.iter().any(|n| n.channel.id == id && n.channel.has_password);
                            (id, locked)
                        });
                        if out.start_channel.is_some_and(|(_, locked)| locked) {
                            out.start_password = password;
                        }
                    }
                }
                self.count_talkers();
                out.tree = true;
                out.header = true;
            }
            Event::Talking { client_id, talking, whisper } => {
                let refused = whisper && !self.allow_whispers;
                let talking = talking && !refused;
                let whisper = whisper && !refused;
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
                    let uid = self.view.as_ref().and_then(|v| v.client(client_id)).map(|c| c.uid.clone());
                    self.reply_to = uid.map(|uid| (client_id, uid));
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
                    TextTarget::Client(to) if mine => {
                        let peer = self.view.as_ref().and_then(|view| view.client(to)).map(|client| client.nickname.clone());
                        format!("to {}", peer.unwrap_or_else(|| "someone".to_string()))
                    }
                    TextTarget::Client(_) => {
                        if self.peer.is_none() {
                            let uid = self
                                .view
                                .as_ref()
                                .and_then(|view| view.client(from_id))
                                .map(|client| client.uid.clone())
                                .unwrap_or_default();
                            self.peer = Some(Peer { id: from_id, uid, name: from_name.clone() });
                            out.header = true;
                        }
                        format!("{from_name} (private)")
                    }
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
                out.groups = true;
            }
            Event::Icon { id, data } => {
                if self.trace {
                    match &data {
                        Ok(bytes) => self.system(&format!("Icon {id} arrived, {} bytes", bytes.len())),
                        Err(reason) => self.system(&format!("Icon {id} could not be fetched: {reason}")),
                    }
                }
                out.icon = Some((id, data));
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
                } else if !self.leaving && self.was_connected && should_retry(&reason) {
                    self.retries += 1;
                    let wait = retry_delay(self.retries);
                    self.retry_at = Some(Instant::now() + wait);
                    self.phase = Phase::Connecting;
                    self.state_text = format!("Connection lost. Trying again in {} s", wait.as_secs());
                    self.request.password = self.kept_password.clone();
                    if self.last_channel.1 != 0 {
                        (self.request.channel, self.request.channel_id) = self.last_channel.clone();
                    }
                    let line = format!("{}. Trying again in {} s.", sentence(&reason), wait.as_secs());
                    self.system(&line);
                    out.retry = true;
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
        marlin.icons = vec![100, 300, 452340182, 2154984321, 7];
        let mut coralline = person(9, 1, "Coralline");
        coralline.input_muted = true;
        coralline.away = true;
        let mut headless = person(11, 4, "Jukebox");
        headless.output_hardware = false;
        let mut query = person(12, 4, "serveradmin");
        query.is_query = true;
        let mut locked = channel(3, 0, "Squad Alpha");
        locked.has_password = true;
        locked.icon = 2154984321;
        let mut banner = channel(20, 0, "[cspacer]Reef Runners");
        banner.icon = 452340182;
        let mut music = channel(2, 0, "Radio");
        music.codec = CODEC_OPUS_MUSIC;
        ServerView {
            server: ServerInfo { name: "Reef Runners".into(), ..ServerInfo::default() },
            own_id: 7,
            own_channel: 1,
            channels: vec![
                ChannelNode { channel: banner, depth: 0, clients: vec![] },
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
        assert_eq!(rows[6].icons, vec![2154984321]);
        assert_eq!(rows[4].icons, vec![100, 300, 452340182, 2154984321]);
        assert!(rows[0].icons.is_empty() && rows[1].icons.is_empty() && rows[2].icons.is_empty());
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

    fn branch_view() -> ServerView {
        let node = |id: u64, parent: u64, depth: u32, name: &str, clients: Vec<ClientInfo>| ChannelNode {
            channel: channel(id, parent, name),
            depth,
            clients,
        };
        ServerView {
            server: ServerInfo::default(),
            own_id: 7,
            own_channel: 30,
            channels: vec![
                node(10, 0, 0, "Empty branch", vec![]),
                node(11, 10, 1, "Empty child", vec![]),
                node(12, 11, 2, "Empty grandchild", vec![]),
                node(20, 0, 0, "Busy branch", vec![]),
                node(21, 20, 1, "Busy child", vec![person(8, 21, "Marlin")]),
                node(30, 0, 0, "Alone", vec![person(7, 30, "Minnow")]),
                node(40, 0, 0, "Bare", vec![]),
                node(50, 0, 0, "[cspacer]Divider", vec![]),
                node(51, 50, 1, "Under a divider", vec![]),
            ],
        }
    }

    fn names(rows: &[RowData]) -> Vec<&str> {
        rows.iter().map(|row| row.text.as_str()).collect()
    }

    #[test]
    fn channels_fold_and_unfold() {
        let view = sample_view();
        let open = build_rows_folded(&view, true, &Folds::default());
        assert_eq!(open, build_rows(&view, true));
        assert!(open[1].foldable && !open[1].folded);
        assert!(!open[6].foldable);
        assert!(open[7].foldable && open[8].foldable);
        assert!(!open[0].foldable && !open[5].foldable);

        let mut folds = Folds::default();
        toggle_fold(&view, &mut folds, 1);
        let rows = build_rows_folded(&view, true, &folds);
        assert_eq!(rows.len(), open.len() - 3);
        assert_eq!((rows[1].text.as_str(), rows[1].count), ("Lobby", 3));
        assert!(rows[1].folded && rows[1].talking && rows[1].current);
        assert_eq!(rows[2].kind, RowKind::SpacerLine);
        let mut quiet = sample_view();
        quiet.channels[1].clients[2].talking = false;
        assert!(build_rows_folded(&quiet, true, &folds)[1].talking);
        assert!(!build_rows_folded(&quiet, false, &folds)[1].talking);

        toggle_fold(&view, &mut folds, 2);
        let rows = build_rows_folded(&view, true, &folds);
        let radio = rows.iter().find(|row| row.text == "Radio").unwrap();
        assert!(radio.folded && radio.count == 2 && !radio.talking);
        assert!(!rows.iter().any(|row| row.text == "Nested" || row.text == "Jukebox"));

        toggle_fold(&view, &mut folds, 1);
        toggle_fold(&view, &mut folds, 3);
        toggle_fold(&view, &mut folds, 20);
        toggle_fold(&view, &mut folds, 999);
        assert_eq!(folds.chosen.get(&1), Some(&false));
        assert!(!folds.chosen.contains_key(&3) && !folds.chosen.contains_key(&20) && !folds.chosen.contains_key(&999));
        assert_eq!(build_rows_folded(&view, true, &folds)[2].text, "Coralline");
    }

    #[test]
    fn empty_branches_start_folded_when_asked() {
        let view = branch_view();
        let all = build_rows_folded(&view, false, &Folds::default());
        assert_eq!(all.len(), 11);

        let mut folds = Folds { mode: FoldMode::Empty, ..Folds::default() };
        let rows = build_rows_folded(&view, false, &folds);
        assert_eq!(
            names(&rows),
            vec!["Empty branch", "Busy branch", "Busy child", "Marlin", "Alone", "Minnow", "Bare", "Divider", "Under a divider"]
        );
        assert!(rows[0].foldable && rows[0].folded && rows[0].count == 0 && !rows[0].current);
        assert!(rows[1].foldable && !rows[1].folded);
        assert!(rows[4].foldable && !rows[4].folded && rows[4].current);
        assert!(!rows[6].foldable && !rows[7].foldable && !rows[8].foldable);

        toggle_fold(&view, &mut folds, 10);
        let rows = build_rows_folded(&view, false, &folds);
        assert_eq!(names(&rows)[..3], ["Empty branch", "Empty child", "Busy branch"]);
        assert!(rows[1].folded);

        let mut busy = branch_view();
        busy.channels[2].clients.push(person(9, 12, "Coralline"));
        let rows = build_rows_folded(&busy, false, &Folds { mode: FoldMode::Empty, ..Folds::default() });
        assert_eq!(names(&rows)[..4], ["Empty branch", "Empty child", "Empty grandchild", "Coralline"]);
    }

    #[test]
    fn everything_can_start_folded_except_the_way_to_me() {
        let mut view = branch_view();
        let mut folds = Folds { mode: FoldMode::All, ..Folds::default() };
        let rows = build_rows_folded(&view, false, &folds);
        assert_eq!(
            names(&rows),
            vec!["Empty branch", "Busy branch", "Alone", "Minnow", "Bare", "Divider", "Under a divider"]
        );
        assert!(rows[0].folded && rows[1].folded && rows[1].count == 1 && !rows[2].folded && rows[2].current);

        view.own_channel = 21;
        let rows = build_rows_folded(&view, false, &folds);
        assert_eq!(
            names(&rows),
            vec!["Empty branch", "Busy branch", "Busy child", "Marlin", "Alone", "Bare", "Divider", "Under a divider"]
        );
        assert!(!rows[1].folded && !rows[2].folded && rows[4].folded && rows[4].count == 1);

        toggle_fold(&view, &mut folds, 10);
        toggle_fold(&view, &mut folds, 20);
        let rows = build_rows_folded(&view, false, &folds);
        assert_eq!(names(&rows), vec!["Empty branch", "Empty child", "Busy branch", "Alone", "Bare", "Divider", "Under a divider"]);
        assert!(rows[1].folded && rows[2].folded && rows[2].current);
        assert!(reveal(&view, &mut folds, 21));
        assert!(!reveal(&view, &mut folds, 21));
        assert!(names(&build_rows_folded(&view, false, &folds)).contains(&"Marlin"));

        let open = Folds { mode: FoldMode::Open, ..Folds::default() };
        assert_eq!(build_rows_folded(&view, false, &open).len(), 11);
        assert_eq!(
            [0, 1, 2, 7, -1].map(FoldMode::from_index),
            [FoldMode::Open, FoldMode::Empty, FoldMode::All, FoldMode::Empty, FoldMode::Empty]
        );
    }

    #[test]
    fn folding_choices_are_kept_for_channels_that_still_exist() {
        let view = branch_view();
        let mut folds = Folds::default();
        folds.chosen.insert(20, true);
        folds.chosen.insert(10, false);
        folds.chosen.insert(999, true);
        assert_eq!(remembered(&folds, Some(&view)), BTreeMap::from([(10, false), (20, true)]));
        assert_eq!(remembered(&folds, None).len(), 3);
        for id in 0..2000u64 {
            folds.chosen.insert(5000 + id, true);
        }
        assert_eq!(remembered(&folds, None).len(), MAX_REMEMBERED_FOLDS);
        assert!(remembered(&Folds::default(), Some(&view)).is_empty());
    }

    #[test]
    fn joining_a_channel_opens_the_way_to_it() {
        let mut view = branch_view();
        let mut folds = Folds::default();
        toggle_fold(&view, &mut folds, 20);
        toggle_fold(&view, &mut folds, 30);
        assert_eq!(names(&build_rows_folded(&view, false, &folds)), vec![
            "Empty branch", "Empty child", "Empty grandchild", "Busy branch", "Alone", "Bare", "Divider", "Under a divider"
        ]);
        let folded = build_rows_folded(&view, true, &folds);
        assert!(folded[4].folded && folded[4].current && folded[4].talking && folded[4].count == 1);

        view.own_channel = 21;
        let hidden = build_rows_folded(&view, false, &folds);
        assert!(hidden[3].folded && hidden[3].current);
        assert!(reveal(&view, &mut folds, 21));
        assert!(!folds.chosen.contains_key(&20));
        assert_eq!(folds.chosen.get(&30), Some(&true));
        assert!(names(&build_rows_folded(&view, false, &folds)).contains(&"Marlin"));
        assert!(!reveal(&view, &mut folds, 777));

        let mut s = session();
        s.apply(Event::Connected {
            client_id: 7,
            server: ServerInfo { uid: "serverA".into(), ..ServerInfo::default() },
        });
        assert_eq!(s.server_uid, "serverA");
        s.folds = Folds::default();
        s.folds.chosen.insert(20, true);
        let mut first = branch_view();
        first.own_channel = 30;
        assert!(!s.apply(Event::View(first)).folds);
        assert!(s.apply(Event::View(view.clone())).folds);
        assert!(!s.folds.chosen.contains_key(&20));
        assert!(!s.apply(Event::View(view)).folds);
    }

    #[test]
    fn private_messages_name_who_they_are_with() {
        let mut s = session();
        s.apply(Event::Connected { client_id: 7, server: ServerInfo::default() });
        s.apply(Event::View(sample_view()));
        let out = s.apply(Event::TextMessage {
            target: TextTarget::Client(7),
            from_id: 8,
            from_name: "Marlin".into(),
            text: "psst".into(),
        });
        assert!(out.header);
        assert_eq!(s.peer.as_ref().map(|peer| (peer.id, peer.name.as_str())), Some((8, "Marlin")));
        assert_eq!(s.chat.back().unwrap().name, "Marlin (private)");
        s.apply(Event::TextMessage {
            target: TextTarget::Client(8),
            from_id: 7,
            from_name: "Minnow".into(),
            text: "yes?".into(),
        });
        let last = s.chat.back().unwrap();
        assert_eq!((last.kind, last.name.as_str()), (ChatKind::Mine, "to Marlin"));
        s.apply(Event::TextMessage {
            target: TextTarget::Client(7),
            from_id: 9,
            from_name: "Coralline".into(),
            text: "hi".into(),
        });
        assert_eq!(s.peer.as_ref().map(|peer| peer.id), Some(8), "a second sender does not take over the reply");
    }

    #[test]
    fn a_lost_connection_is_tried_again_but_a_kick_is_not() {
        assert!(should_retry("connection lost (the server stopped responding)"));
        assert!(should_retry("connection lost (a packet was never acknowledged)"));
        assert!(should_retry("server is shutting down (maintenance)"));
        assert!(should_retry("server stopped"));
        assert!(should_retry("no response from reef.example.net:9987 (is a TeamSpeak 3 server running there?)"));
        assert!(should_retry("nothing is listening on reef.example.net:9987 (port unreachable)"));
        assert!(should_retry("cannot find the address of reef.example.net"));
        assert!(!should_retry("kicked from the server by Marlin (bye)"));
        assert!(!should_retry("banned from the server"));
        assert!(!should_retry("the server refused the connection: invalid server password"));
        assert!(!should_retry("disconnected"));
        assert!(!should_retry("left"));
        let waits: Vec<u64> = (1..=7).map(|attempt| retry_delay(attempt).as_secs()).collect();
        assert_eq!(waits, vec![2, 4, 8, 15, 30, 30, 30]);

        let mut s = session();
        s.apply(Event::Connected { client_id: 7, server: ServerInfo::default() });
        s.apply(Event::View(sample_view()));
        assert_eq!(s.request.password, "");
        let out = s.apply(Event::Disconnected { reason: "connection lost (the server stopped responding)".into() });
        assert!(out.retry && out.closed.is_none() && out.forget_all);
        assert_eq!((s.phase.clone(), s.retries), (Phase::Connecting, 1));
        assert!(s.retry_at.is_some() && s.view.is_none());
        assert_eq!((s.request.channel.as_str(), s.request.channel_id), ("Lobby", 1));
        assert_eq!(s.request.password, "hunter2", "the password is kept in memory for the next try");
        assert!(s.chat.back().unwrap().text.contains("Trying again in 2 s"));

        let out = s.apply(Event::Disconnected { reason: "no response from reef.example.net:9987".into() });
        assert!(out.retry);
        assert_eq!(s.retries, 2);
        s.apply(Event::Connected { client_id: 9, server: ServerInfo::default() });
        assert_eq!((s.retries, s.retry_at), (0, None));

        let out = s.apply(Event::Disconnected { reason: "kicked from the server by Marlin".into() });
        assert!(!out.retry && out.closed.is_some());

        let mut fresh = session();
        let out = fresh.apply(Event::Disconnected { reason: "no response from reef.example.net:9987".into() });
        assert!(!out.retry && out.closed.is_some(), "a first attempt that fails is not repeated");

        let mut going = session();
        going.apply(Event::Connected { client_id: 7, server: ServerInfo::default() });
        going.leaving = true;
        let out = going.apply(Event::Disconnected { reason: "connection lost (the server stopped responding)".into() });
        assert!(!out.retry && out.closed.is_some());
    }

    #[test]
    fn whispers_can_be_refused() {
        let mut s = session();
        s.allow_whispers = false;
        s.apply(Event::Connected { client_id: 7, server: ServerInfo::default() });
        let mut view = sample_view();
        view.channels[1].clients[2].talking = false;
        s.apply(Event::View(view));
        let lines = s.chat.len();
        let out = s.apply(Event::Talking { client_id: 8, talking: true, whisper: true });
        assert_eq!(out.whisper_from, None);
        assert_eq!(s.chat.len(), lines);
        let rows = build_rows(s.view.as_ref().unwrap(), false);
        assert!(rows.iter().any(|row| row.text == "Marlin" && !row.talking && !row.whispering));
        assert_eq!(s.reply_to, None);
        let mut whispered = sample_view();
        whispered.channels[1].clients[2].whispering = true;
        s.apply(Event::View(whispered));
        assert_eq!(s.talkers, 0);
        let rows = build_rows(s.view.as_ref().unwrap(), false);
        assert!(rows.iter().any(|row| row.text == "Marlin" && !row.talking && !row.whispering && row.tag.is_empty()));
        s.apply(Event::Talking { client_id: 8, talking: true, whisper: false });
        assert!(build_rows(s.view.as_ref().unwrap(), false).iter().any(|row| row.text == "Marlin" && row.talking));

        s.set_allow_whispers(true);
        let mut named = sample_view();
        named.channels[1].clients[2].uid = "uidMarlin".into();
        s.apply(Event::View(named));
        s.apply(Event::Talking { client_id: 8, talking: true, whisper: true });
        assert_eq!(s.reply_to, Some((8, "uidMarlin".to_string())));
        assert_eq!(s.talkers, 1);
        s.set_allow_whispers(false);
        assert_eq!(s.talkers, 0);
        assert!(build_rows(s.view.as_ref().unwrap(), false).iter().all(|row| !row.whispering));

        let out = s.apply(Event::Groups {
            server_groups: vec![Group { id: 6, name: "Server Admin".into(), kind: 1, sort: 0, icon: 300 }],
            channel_groups: vec![],
        });
        assert!(out.groups && s.server_groups.len() == 1);
    }

    #[test]
    fn the_start_channel_is_found_by_path_then_by_number() {
        let mut view = branch_view();
        view.channels[2].channel.name = "Up/Down".into();
        assert_eq!(channel_path(&view, 10).as_deref(), Some("Empty branch"));
        assert_eq!(channel_path(&view, 12).as_deref(), Some("Empty branch/Empty child/Up\\/Down"));
        assert_eq!(channel_path(&view, 999), None);
        assert_eq!(find_start_channel(&view, "Busy branch/Busy child", 0), Some(21));
        assert_eq!(find_start_channel(&view, "Empty branch/Empty child/Up\\/Down", 0), Some(12));
        assert_eq!(find_start_channel(&view, "Busy child", 0), None);
        assert_eq!(find_start_channel(&view, "Renamed since", 21), Some(21));
        assert_eq!(find_start_channel(&view, "Busy branch", 21), Some(20));
        assert_eq!(find_start_channel(&view, "", 0), None);
        assert_eq!(find_start_channel(&view, "Gone", 999), None);

        let request = ConnectRequest {
            address: "reef.example.net".into(),
            channel: "Busy branch/Busy child".into(),
            channel_id: 21,
            ..ConnectRequest::default()
        };
        let mut s = Session::new(1, request, false);
        s.apply(Event::Connected { client_id: 7, server: ServerInfo::default() });
        let mut early = branch_view();
        early.own_channel = 0;
        assert_eq!(s.apply(Event::View(early)).start_channel, None);
        assert_eq!(s.apply(Event::View(branch_view())).start_channel, Some((21, false)));
        assert_eq!(s.apply(Event::View(branch_view())).start_channel, None);

        let mut locked = branch_view();
        locked.channels[4].channel.has_password = true;
        let mut again = Session::new(2, s.request.clone(), false);
        again.apply(Event::Connected { client_id: 7, server: ServerInfo::default() });
        assert_eq!(again.apply(Event::View(locked)).start_channel, Some((21, true)));

        let mut there = branch_view();
        there.own_channel = 21;
        let mut arrived = Session::new(3, s.request.clone(), false);
        arrived.apply(Event::Connected { client_id: 7, server: ServerInfo::default() });
        assert_eq!(arrived.apply(Event::View(there)).start_channel, None);

        let mut plain = session();
        plain.apply(Event::Connected { client_id: 7, server: ServerInfo::default() });
        assert_eq!(plain.apply(Event::View(branch_view())).start_channel, None);
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
