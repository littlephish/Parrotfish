pub mod book;
mod conn;
pub mod filetransfer;
pub mod spacer;
mod stats;

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use ps_identity::Identity;
use ps_protocol::command::Command;

pub use book::{
    icon_id, is_standard_icon, Channel, ChannelNode, ClientInfo, Group, ServerInfo, ServerView, STANDARD_ICONS,
};
pub use ps_protocol::voice::{CODEC_OPUS_MUSIC, CODEC_OPUS_VOICE};

pub const DEFAULT_PORT: u16 = 9987;
pub const CLIENT_VERSION: &str = "3.?.? [Build: 5680278000]";
pub const CLIENT_PLATFORM: &str = "Windows";
pub const CLIENT_VERSION_SIGN: &str =
    "DX5NIYLvfJEUjuIbCidnoeozxIDRRkpq3I9vVMBmE9L2qnekOoBzSenkzsg2lC9CMv8K5hkEzhr2TYUYSwUXCg==";

pub const ERROR_IDENTITY_LEVEL: u32 = 0x0207;

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub host: String,
    pub port: u16,
    pub identity: Identity,
    pub nickname: String,
    pub server_password: String,
    pub default_channel: String,
    pub default_channel_password: String,
    pub input_muted: bool,
    pub output_muted: bool,
    pub log_commands: bool,
    pub simulated_loss: f32,
    pub filetransfer_port: Option<u16>,
}

impl ConnectOptions {
    pub fn new(host: &str, port: u16, identity: Identity) -> Self {
        let nickname = if identity.nickname.trim().is_empty() {
            "PhishSpeak".to_string()
        } else {
            identity.nickname.clone()
        };
        Self {
            host: host.to_string(),
            port,
            identity,
            nickname,
            server_password: String::new(),
            default_channel: String::new(),
            default_channel_password: String::new(),
            input_muted: false,
            output_muted: false,
            log_commands: false,
            simulated_loss: 0.0,
            filetransfer_port: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Resolving,
    Connecting,
    Handshake,
    Connected,
    Disconnecting,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTarget {
    Client(u16),
    Channel,
    Server,
}

impl TextTarget {
    pub fn mode(self) -> u8 {
        match self {
            TextTarget::Client(_) => 1,
            TextTarget::Channel => 2,
            TextTarget::Server => 3,
        }
    }
}

pub const ERROR_NO_WHISPER_TARGETS: u32 = 0x070c;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperGroup {
    ServerGroup(u64),
    ChannelGroup(u64),
    Commanders,
    Everyone,
}

impl WhisperGroup {
    fn wire(self) -> (u8, u64) {
        match self {
            WhisperGroup::ServerGroup(id) => (0, id),
            WhisperGroup::ChannelGroup(id) => (1, id),
            WhisperGroup::Commanders => (2, 0),
            WhisperGroup::Everyone => (3, 0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperScope {
    AllChannels,
    CurrentChannel,
    ParentChannel,
    AllParentChannels,
    ChannelFamily,
    WholeFamily,
    Subchannels,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhisperTarget {
    List { channels: Vec<u64>, clients: Vec<u16> },
    Group { who: WhisperGroup, scope: WhisperScope },
}

impl WhisperTarget {
    pub fn is_group(&self) -> bool {
        matches!(self, WhisperTarget::Group { .. })
    }

    pub fn header_len(&self) -> usize {
        match self {
            WhisperTarget::List { channels, clients } => {
                ps_protocol::voice::whisper_header_len(channels.len(), clients.len())
            }
            WhisperTarget::Group { .. } => ps_protocol::voice::GROUP_WHISPER_HEADER_LEN,
        }
    }

    pub fn frame_room(&self) -> usize {
        ps_protocol::packet::MAX_C2S_PAYLOAD.saturating_sub(self.header_len())
    }

    pub fn payload(&self, codec: u8, data: &[u8]) -> Option<Vec<u8>> {
        match self {
            WhisperTarget::List { channels, clients } => {
                ps_protocol::voice::encode_c2s_whisper(codec, channels, clients, data)
            }
            WhisperTarget::Group { who, scope } => {
                let (kind, id) = who.wire();
                Some(ps_protocol::voice::encode_c2s_group_whisper(codec, kind, *scope as u8, id, data))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkStats {
    pub ping_ms: f32,
    pub ping_deviation_ms: f32,
    pub packets_resent: u64,
    pub voice_packets_in: u64,
    pub voice_packets_out: u64,
}

#[derive(Debug, Clone)]
pub enum Event {
    Log(String),
    State(ConnectionState),
    Connected { client_id: u16, server: ServerInfo },
    View(ServerView),
    Talking { client_id: u16, talking: bool, whisper: bool },
    TextMessage { target: TextTarget, from_id: u16, from_name: String, text: String },
    Poke { from_name: String, text: String },
    ClientEntered { client: ClientInfo },
    ClientLeft { client: ClientInfo, reason: String },
    ClientMoved { client: ClientInfo, from: u64, to: u64 },
    ServerError { id: u32, message: String, extra: String },
    SecurityLevelRequired(u8),
    Groups { server_groups: Vec<Group>, channel_groups: Vec<Group> },
    Stats(LinkStats),
    Icon { id: u32, data: Result<Vec<u8>, String> },
    Disconnected { reason: String },
}

#[derive(Debug, Clone, Copy)]
pub struct VoicePacket<'a> {
    pub client_id: u16,
    pub voice_id: u16,
    pub codec: u8,
    pub data: &'a [u8],
    pub whisper: bool,
}

pub type VoiceSink = Box<dyn FnMut(VoicePacket<'_>) + Send>;

pub(crate) struct Shared {
    pub connected: AtomicBool,
    pub closed: AtomicBool,
    pub client_id: AtomicU16,
    pub own_channel: AtomicU64,
}

struct HandleInner {
    tx: Sender<conn::Request>,
    shared: Arc<Shared>,
}

impl Drop for HandleInner {
    fn drop(&mut self) {
        let _ = self.tx.send(conn::Request::Disconnect("leaving".into()));
    }
}

#[derive(Clone)]
pub struct ClientHandle {
    inner: Arc<HandleInner>,
}

impl ClientHandle {
    pub fn connect(options: ConnectOptions, events: Sender<Event>, voice: Option<VoiceSink>) -> Self {
        let shared = Arc::new(Shared {
            connected: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            client_id: AtomicU16::new(0),
            own_channel: AtomicU64::new(0),
        });
        let tx = conn::spawn(options, events, voice, shared.clone());
        Self { inner: Arc::new(HandleInner { tx, shared }) }
    }

    pub fn is_connected(&self) -> bool {
        self.inner.shared.connected.load(Ordering::Relaxed)
    }

    pub fn is_closed(&self) -> bool {
        self.inner.shared.closed.load(Ordering::Relaxed)
    }

    pub fn client_id(&self) -> u16 {
        self.inner.shared.client_id.load(Ordering::Relaxed)
    }

    pub fn own_channel(&self) -> u64 {
        self.inner.shared.own_channel.load(Ordering::Relaxed)
    }

    pub fn send_voice(&self, codec: u8, data: &[u8]) {
        if self.is_connected() {
            let _ = self.inner.tx.send(conn::Request::Voice { codec, data: data.to_vec() });
        }
    }

    pub fn send_whisper(&self, target: &WhisperTarget, codec: u8, data: &[u8]) {
        if !self.is_connected() {
            return;
        }
        if let Some(payload) = target.payload(codec, data) {
            let _ = self.inner.tx.send(conn::Request::Whisper { payload, group: target.is_group() });
        }
    }

    pub fn request_icon(&self, id: u32) {
        let _ = self.inner.tx.send(conn::Request::Icon(id));
    }

    pub fn set_channel_commander(&self, on: bool) {
        self.send_command(Command::new("clientupdate").arg("client_is_channel_commander", u8::from(on)));
    }

    pub fn send_command(&self, command: Command) {
        let _ = self.inner.tx.send(conn::Request::Command(command));
    }

    pub fn send_text(&self, target: TextTarget, text: &str) {
        let _ = self.inner.tx.send(conn::Request::Text { target, text: text.to_string() });
    }

    pub fn join_channel(&self, channel_id: u64, password: &str) {
        let mut cmd = Command::new("clientmove")
            .arg("clid", self.client_id())
            .arg("cid", channel_id);
        if !password.is_empty() {
            cmd.push("cpw", ps_crypto::hash_password(password));
        }
        self.send_command(cmd);
    }

    pub fn set_mute_state(&self, input_muted: bool, output_muted: bool) {
        self.send_command(
            Command::new("clientupdate")
                .arg("client_input_muted", u8::from(input_muted))
                .arg("client_output_muted", u8::from(output_muted)),
        );
    }

    pub fn set_away(&self, away: bool, message: &str) {
        self.send_command(
            Command::new("clientupdate")
                .arg("client_away", u8::from(away))
                .arg("client_away_message", message),
        );
    }

    pub fn set_nickname(&self, nickname: &str) {
        self.send_command(Command::new("clientupdate").arg("client_nickname", nickname));
    }

    pub fn disconnect(&self, message: &str) {
        let _ = self.inner.tx.send(conn::Request::Disconnect(message.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_targets_encode_for_the_wire() {
        let list = WhisperTarget::List { channels: vec![1, 9], clients: vec![8] };
        assert!(!list.is_group());
        assert_eq!(list.header_len(), 23);
        assert_eq!(list.frame_room(), ps_protocol::packet::MAX_C2S_PAYLOAD - 23);
        assert_eq!(
            list.payload(4, &[0xAA]).unwrap(),
            vec![0, 0, 4, 2, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 9, 0, 8, 0xAA]
        );
        let wire = |who: WhisperGroup, scope: WhisperScope| {
            WhisperTarget::Group { who, scope }.payload(4, &[]).unwrap()[3..].to_vec()
        };
        assert_eq!(wire(WhisperGroup::ServerGroup(6), WhisperScope::AllChannels), vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 6]);
        assert_eq!(wire(WhisperGroup::ChannelGroup(5), WhisperScope::CurrentChannel), vec![1, 1, 0, 0, 0, 0, 0, 0, 0, 5]);
        assert_eq!(wire(WhisperGroup::Commanders, WhisperScope::ParentChannel), vec![2, 2, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(wire(WhisperGroup::Everyone, WhisperScope::AllParentChannels)[..2], [3, 3]);
        assert_eq!(wire(WhisperGroup::Everyone, WhisperScope::ChannelFamily)[1], 4);
        assert_eq!(wire(WhisperGroup::Everyone, WhisperScope::WholeFamily)[1], 5);
        assert_eq!(wire(WhisperGroup::Everyone, WhisperScope::Subchannels)[1], 6);
        let group = WhisperTarget::Group { who: WhisperGroup::Everyone, scope: WhisperScope::AllChannels };
        assert!(group.is_group());
        assert_eq!((group.header_len(), group.frame_room()), (13, ps_protocol::packet::MAX_C2S_PAYLOAD - 13));
        let crowded = WhisperTarget::List { channels: vec![1; 70], clients: vec![] };
        assert_eq!(crowded.frame_room(), 0);
    }
}
