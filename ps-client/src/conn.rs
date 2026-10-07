use std::collections::{HashMap, HashSet, VecDeque};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use ps_crypto::handshake::{self, ServerHello};
use ps_crypto::keys::{SessionKeys, DUMMY_KEY, DUMMY_NONCE, INIT_MAC};
use ps_crypto::{eax, hash_password};
use ps_protocol::command::Command;
use ps_protocol::fragment::{split_command, CommandQueue, Slot};
use ps_protocol::init::{self, ServerInit, INIT_VERSION};
use ps_protocol::packet::{
    Direction, Header, PacketType, FLAG_NEWPROTOCOL, FLAG_UNENCRYPTED, INIT_PACKET_ID,
    MAX_C2S_PAYLOAD, PACKET_TYPE_COUNT,
};
use ps_protocol::voice::{is_end_of_stream, parse_s2c_voice};
use ps_protocol::window::{PacketCounter, ReceiveWindow};
use rand::RngCore as _;
use sha1::Digest as _;
use sha1::Sha1;

use crate::book::{is_standard_icon, Book};
use crate::filetransfer::{self, ICON_SIZE_LIMIT};
use crate::resolve::{resolve, Found};
use crate::stats::Stats;
use crate::{
    ConnectOptions, ConnectionState, Event, LinkStats, Shared, TextTarget, VoicePacket, VoiceSink,
    CLIENT_PLATFORM, CLIENT_VERSION, CLIENT_VERSION_SIGN, ERROR_IDENTITY_LEVEL,
};

const TICK: Duration = Duration::from_millis(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PACKET_TIMEOUT: Duration = Duration::from_secs(30);
const PING_INTERVAL: Duration = Duration::from_secs(1);
const VIEW_INTERVAL: Duration = Duration::from_millis(50);
const STATS_INTERVAL: Duration = Duration::from_secs(1);
const TALK_TIMEOUT: Duration = Duration::from_millis(400);
const LATE_VOICE: Duration = Duration::from_millis(500);
const DISCONNECT_GRACE: Duration = Duration::from_millis(1500);
const ANNOUNCE_DELAY: Duration = Duration::from_millis(1500);
const INITIAL_RTO_MS: f32 = 500.0;
const MIN_RTO_MS: f32 = 150.0;
const MAX_RTO_MS: f32 = 1000.0;
const ICON_GAP: Duration = Duration::from_millis(500);
const ICON_ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
const ICON_TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);
const ICON_FLOOD_PAUSE: Duration = Duration::from_secs(15);
const ICON_QUEUE_LIMIT: usize = 600;
const ICON_MISS_LIMIT: u32 = 3;
const ICON_GIVE_UP_FOR: Duration = Duration::from_secs(600);
const ERROR_FLOODING: u32 = 0x020c;

pub(crate) enum Request {
    Datagram(Vec<u8>),
    PortUnreachable,
    Command(Command),
    Text { target: TextTarget, text: String },
    Voice { codec: u8, data: Vec<u8> },
    Whisper { payload: Vec<u8>, group: bool },
    Icon(u32),
    Disconnect(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Init(u8),
    WaitInitServer,
    Connected,
    Disconnecting,
}

struct IconTransfer {
    transfer: u16,
    icon: u32,
    asked: Instant,
    return_code: u32,
}

struct Pending {
    ptype: PacketType,
    id: u16,
    raw: Vec<u8>,
    first_sent: Instant,
    last_sent: Instant,
    tries: u32,
}

fn is_older(voice_id: u16, marker: u16) -> bool {
    (voice_id.wrapping_sub(marker) as i16) < 0
}

pub(crate) fn spawn(
    options: ConnectOptions,
    events: Sender<Event>,
    voice: Option<VoiceSink>,
    shared: Arc<Shared>,
) -> Sender<Request> {
    let (tx, rx) = mpsc::channel();
    let actor_tx = tx.clone();
    std::thread::Builder::new()
        .name("ps-client".into())
        .spawn(move || run(options, events, voice, shared, actor_tx, rx))
        .expect("failed to spawn client thread");
    tx
}

fn run(
    options: ConnectOptions,
    events: Sender<Event>,
    voice: Option<VoiceSink>,
    shared: Arc<Shared>,
    tx: Sender<Request>,
    rx: Receiver<Request>,
) {
    let _ = events.send(Event::State(ConnectionState::Resolving));
    let reason = match open(&options) {
        Ok((socket, found)) => {
            let _ = events
                .send(Event::Log(format!("Connecting to {} (found through {})", found.addr, found.how)));
            let reader = socket.try_clone();
            match reader {
                Ok(reader) => {
                    let reader_shared = shared.clone();
                    let reader_tx = tx.clone();
                    let _ = std::thread::Builder::new()
                        .name("ps-client-rx".into())
                        .spawn(move || read_loop(reader, reader_tx, reader_shared));
                    drop(tx);
                    let mut conn = Conn::new(options, events.clone(), voice, shared.clone(), socket);
                    conn.run(rx)
                }
                Err(e) => format!("cannot clone socket: {e}"),
            }
        }
        Err(e) => e,
    };
    shared.connected.store(false, Ordering::Relaxed);
    shared.closed.store(true, Ordering::Relaxed);
    let _ = events.send(Event::State(ConnectionState::Disconnected));
    let _ = events.send(Event::Disconnected { reason });
}

fn open(options: &ConnectOptions) -> Result<(UdpSocket, Found), String> {
    if options.identity.private_key.is_none() {
        return Err("the selected identity has no private key".into());
    }
    let found = resolve(&options.host, options.port)?;
    let addr = found.addr;
    let bind = if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let socket = UdpSocket::bind(bind).map_err(|e| format!("cannot open UDP socket: {e}"))?;
    socket.connect(addr).map_err(|e| format!("cannot reach {addr}: {e}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|e| format!("socket setup failed: {e}"))?;
    Ok((socket, found))
}

fn read_loop(socket: UdpSocket, tx: Sender<Request>, shared: Arc<Shared>) {
    let mut buf = [0u8; 2048];
    while !shared.closed.load(Ordering::Relaxed) {
        match socket.recv(&mut buf) {
            Ok(n) => {
                if tx.send(Request::Datagram(buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) if e.kind() == ErrorKind::ConnectionReset => {
                if tx.send(Request::PortUnreachable).is_err() {
                    break;
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

struct Conn {
    opts: ConnectOptions,
    events: Sender<Event>,
    voice: Option<VoiceSink>,
    shared: Arc<Shared>,
    socket: UdpSocket,
    phase: Phase,
    finished: Option<String>,
    started: Instant,
    last_rx: Instant,
    received_any: bool,
    keys: Option<SessionKeys>,
    client_id: u16,
    alpha: [u8; 10],
    out_counters: [PacketCounter; PACKET_TYPE_COUNT],
    in_windows: [ReceiveWindow; PACKET_TYPE_COUNT],
    queues: [CommandQueue; 2],
    init_pending: Option<Pending>,
    resend: Vec<Pending>,
    srtt_ms: Option<f32>,
    rttvar_ms: f32,
    rto_ms: f32,
    last_ping: Option<(u16, Instant)>,
    next_ping: Instant,
    book: Book,
    view_dirty: bool,
    last_view: Instant,
    last_stats: Instant,
    talking: HashMap<u16, Instant>,
    voice_ended: HashMap<u16, (u16, Instant)>,
    stats: Stats,
    return_code: u32,
    disconnect_deadline: Option<Instant>,
    disconnect_packet: Option<u16>,
    voice_errors: u64,
    announce_after: Instant,
    icon_queue: VecDeque<u32>,
    icon_pending: HashSet<u32>,
    icon_active: Option<IconTransfer>,
    icon_job: Option<Receiver<Result<Vec<u8>, String>>>,
    next_transfer: u16,
    next_icon_at: Instant,
    icon_misses: u32,
    icons_off_until: Option<Instant>,
}

impl Conn {
    fn new(
        opts: ConnectOptions,
        events: Sender<Event>,
        voice: Option<VoiceSink>,
        shared: Arc<Shared>,
        socket: UdpSocket,
    ) -> Self {
        let now = Instant::now();
        let mut out_counters = [PacketCounter::default(); PACKET_TYPE_COUNT];
        out_counters[PacketType::Command.index()] = PacketCounter::starting_at(1);
        Self {
            opts,
            events,
            voice,
            shared,
            socket,
            phase: Phase::Init(0),
            finished: None,
            started: now,
            last_rx: now,
            received_any: false,
            keys: None,
            client_id: 0,
            alpha: [0; 10],
            out_counters,
            in_windows: [ReceiveWindow::default(); PACKET_TYPE_COUNT],
            queues: [CommandQueue::default(), CommandQueue::default()],
            init_pending: None,
            resend: Vec::new(),
            srtt_ms: None,
            rttvar_ms: 0.0,
            rto_ms: INITIAL_RTO_MS,
            last_ping: None,
            next_ping: now + PING_INTERVAL,
            book: Book::default(),
            view_dirty: false,
            last_view: now,
            last_stats: now,
            talking: HashMap::new(),
            voice_ended: HashMap::new(),
            stats: Stats::new(),
            return_code: 0,
            disconnect_deadline: None,
            disconnect_packet: None,
            voice_errors: 0,
            announce_after: now + CONNECT_TIMEOUT + PACKET_TIMEOUT,
            icon_queue: VecDeque::new(),
            icon_pending: HashSet::new(),
            icon_active: None,
            icon_job: None,
            next_transfer: 1,
            next_icon_at: now,
            icon_misses: 0,
            icons_off_until: None,
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn log(&self, text: impl Into<String>) {
        self.emit(Event::Log(text.into()));
    }

    fn fail(&mut self, reason: impl Into<String>) {
        if self.finished.is_none() {
            self.finished = Some(reason.into());
        }
    }

    fn run(&mut self, rx: Receiver<Request>) -> String {
        self.emit(Event::State(ConnectionState::Connecting));
        self.send_init0();
        while self.finished.is_none() {
            match rx.recv_timeout(TICK) {
                Ok(request) => self.handle(request),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => self.fail("client handle dropped"),
            }
            self.tick();
        }
        self.finished.clone().unwrap_or_default()
    }

    fn handle(&mut self, request: Request) {
        match request {
            Request::Datagram(raw) => self.on_datagram(&raw),
            Request::PortUnreachable => {
                if matches!(self.phase, Phase::Init(_)) && !self.received_any {
                    self.fail(format!(
                        "nothing is listening on {}:{} (port unreachable)",
                        self.opts.host, self.opts.port
                    ));
                }
            }
            Request::Command(cmd) => {
                if self.phase == Phase::Connected {
                    self.send_tracked(cmd);
                }
            }
            Request::Text { target, text } => {
                if self.phase == Phase::Connected {
                    let id = match target {
                        TextTarget::Client(id) => id as u64,
                        TextTarget::Channel => self.book.own_channel(),
                        TextTarget::Server => self.book.server.virtual_server_id.max(1),
                    };
                    self.send_tracked(
                        Command::new("sendtextmessage")
                            .arg("targetmode", target.mode())
                            .arg("target", id)
                            .arg("msg", text),
                    );
                }
            }
            Request::Voice { codec, data } => self.send_voice(codec, &data),
            Request::Whisper { payload, group } => {
                if self.phase == Phase::Connected && payload.len() <= MAX_C2S_PAYLOAD {
                    let flags = if group { FLAG_NEWPROTOCOL } else { 0 };
                    self.send_packet(PacketType::VoiceWhisper, flags, &payload);
                }
            }
            Request::Icon(id) => self.queue_icon(id),
            Request::Disconnect(message) => self.begin_disconnect(&message),
        }
    }

    fn begin_disconnect(&mut self, message: &str) {
        match self.phase {
            Phase::Connected => {
                self.phase = Phase::Disconnecting;
                self.shared.connected.store(false, Ordering::Relaxed);
                self.emit(Event::State(ConnectionState::Disconnecting));
                let cmd = Command::new("clientdisconnect").arg("reasonid", 8).arg("reasonmsg", message);
                self.disconnect_packet = self.send_command(&cmd);
                self.disconnect_deadline = Some(Instant::now() + DISCONNECT_GRACE);
            }
            Phase::Disconnecting => {}
            _ => self.fail("connection attempt cancelled"),
        }
    }

    fn drop_for_test(&self) -> bool {
        self.opts.simulated_loss > 0.0 && rand::random::<f32>() < self.opts.simulated_loss
    }

    fn send_raw(&mut self, ptype: PacketType, raw: &[u8]) {
        self.stats.sent(ptype, raw.len());
        if self.drop_for_test() {
            return;
        }
        let _ = self.socket.send(raw);
    }

    fn send_init(&mut self, payload: Vec<u8>) {
        let mut header = Header::c2s(PacketType::Init1, FLAG_UNENCRYPTED, INIT_PACKET_ID, 0);
        header.mac = INIT_MAC;
        let raw = header.build(&payload);
        self.send_raw(PacketType::Init1, &raw);
        let now = Instant::now();
        self.init_pending = Some(Pending {
            ptype: PacketType::Init1,
            id: INIT_PACKET_ID,
            raw,
            first_sent: now,
            last_sent: now,
            tries: 0,
        });
    }

    fn send_init0(&mut self) {
        let mut random = [0u8; 4];
        rand::rngs::OsRng.fill_bytes(&mut random);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        self.phase = Phase::Init(0);
        self.send_init(init::init0(INIT_VERSION, timestamp, random));
    }

    fn send_packet(&mut self, ptype: PacketType, flags: u8, payload: &[u8]) -> (u16, Vec<u8>) {
        let (id, generation) = self.out_counters[ptype.index()].next();
        let encrypt = match ptype {
            PacketType::Command | PacketType::CommandLow | PacketType::Ack | PacketType::AckLow => true,
            PacketType::Voice | PacketType::VoiceWhisper => self.book.voice_encryption(),
            PacketType::Ping | PacketType::Pong | PacketType::Init1 => false,
        };
        let mut data = payload.to_vec();
        if ptype.is_voice() && data.len() >= 2 {
            data[..2].copy_from_slice(&id.to_be_bytes());
        }
        let flags = if encrypt { flags } else { flags | FLAG_UNENCRYPTED };
        let mut header = Header::c2s(ptype, flags, id, self.client_id);
        if encrypt {
            let (key, nonce) = match &self.keys {
                Some(keys) => keys.key_nonce(false, ptype as u8, id, generation),
                None => (DUMMY_KEY, DUMMY_NONCE),
            };
            header.mac = eax::encrypt(&key, &nonce, &header.meta(), &mut data);
        } else if let Some(keys) = &self.keys {
            header.mac = keys.shared_mac;
        }
        let raw = header.build(&data);
        self.send_raw(ptype, &raw);
        (id, raw)
    }

    fn send_command(&mut self, cmd: &Command) -> Option<u16> {
        let text = cmd.build();
        if self.opts.log_commands {
            self.log(format!("> {text}"));
        }
        let mut last = None;
        for fragment in split_command(text.as_bytes(), MAX_C2S_PAYLOAD) {
            let (id, raw) =
                self.send_packet(PacketType::Command, FLAG_NEWPROTOCOL | fragment.flags, &fragment.data);
            let now = Instant::now();
            self.resend.push(Pending {
                ptype: PacketType::Command,
                id,
                raw,
                first_sent: now,
                last_sent: now,
                tries: 0,
            });
            last = Some(id);
        }
        last
    }

    fn send_tracked(&mut self, mut cmd: Command) {
        self.return_code = self.return_code.wrapping_add(1);
        cmd.push("return_code", self.return_code);
        self.send_command(&cmd);
    }

    fn send_ack(&mut self, for_type: PacketType, id: u16) {
        let ack = if for_type == PacketType::Command { PacketType::Ack } else { PacketType::AckLow };
        self.send_packet(ack, 0, &id.to_be_bytes());
    }

    fn send_voice(&mut self, codec: u8, data: &[u8]) {
        if self.phase != Phase::Connected {
            return;
        }
        let mut payload = Vec::with_capacity(3 + data.len());
        payload.extend_from_slice(&[0, 0, codec]);
        payload.extend_from_slice(data);
        if payload.len() > MAX_C2S_PAYLOAD {
            return;
        }
        self.send_packet(PacketType::Voice, 0, &payload);
    }

    fn decrypt(&self, header: &Header, ptype: PacketType, generation: u32, data: &mut [u8]) -> bool {
        if header.has(FLAG_UNENCRYPTED) {
            return match &self.keys {
                Some(keys) => header.mac == keys.shared_mac,
                None => false,
            };
        }
        let meta = header.meta();
        match &self.keys {
            Some(keys) => {
                let (key, nonce) = keys.key_nonce(true, ptype as u8, header.packet_id, generation);
                if eax::decrypt(&key, &nonce, &meta, data, &header.mac) {
                    return true;
                }
                ptype == PacketType::Ack
                    && eax::decrypt(&DUMMY_KEY, &DUMMY_NONCE, &meta, data, &header.mac)
            }
            None => eax::decrypt(&DUMMY_KEY, &DUMMY_NONCE, &meta, data, &header.mac),
        }
    }

    fn on_datagram(&mut self, raw: &[u8]) {
        if self.drop_for_test() {
            return;
        }
        let Some((header, payload)) = Header::parse(Direction::S2C, raw) else {
            return;
        };
        let Some(ptype) = header.packet_type() else {
            return;
        };
        self.stats.received(ptype, raw.len());
        match ptype {
            PacketType::Init1 => self.on_init(&header, payload),
            PacketType::Command | PacketType::CommandLow => self.on_command_packet(ptype, &header, payload),
            PacketType::Ack | PacketType::AckLow => self.on_ack(ptype, &header, payload),
            PacketType::Ping => self.on_ping(&header),
            PacketType::Pong => self.on_pong(&header, payload),
            PacketType::Voice | PacketType::VoiceWhisper => self.on_voice(ptype, &header, payload),
        }
    }

    fn mark_received(&mut self) {
        self.last_rx = Instant::now();
        self.received_any = true;
    }

    fn on_init(&mut self, header: &Header, payload: &[u8]) {
        let Phase::Init(step) = self.phase else {
            return;
        };
        if header.mac != INIT_MAC {
            return;
        }
        self.mark_received();
        match init::parse_server_init(payload) {
            Ok(ServerInit::Step1 { server_cookie, echo }) => {
                if step == 0 {
                    self.phase = Phase::Init(2);
                    self.send_init(init::init2(INIT_VERSION, &server_cookie, &echo));
                }
            }
            Ok(ServerInit::Step3(puzzle)) => {
                if step != 2 {
                    return;
                }
                let solution = match handshake::solve_puzzle(&puzzle.x, &puzzle.n, puzzle.level) {
                    Ok(y) => y,
                    Err(e) => {
                        self.fail(e.to_string());
                        return;
                    }
                };
                self.alpha = handshake::random_alpha();
                let clientinitiv = Command::new("clientinitiv")
                    .arg("alpha", B64.encode(self.alpha))
                    .arg("omega", self.opts.identity.public_key_der_base64())
                    .arg("ot", 1)
                    .flag("ip")
                    .build();
                self.phase = Phase::Init(4);
                self.emit(Event::State(ConnectionState::Handshake));
                self.send_init(init::init4(INIT_VERSION, &puzzle, &solution, &clientinitiv));
            }
            Ok(ServerInit::Restart) => self.send_init0(),
            Ok(ServerInit::Error(code)) => {
                self.fail(format!("the server refused the connection (init error {code:#06x})"))
            }
            Err(e) => self.log(format!("Ignoring malformed init packet: {e}")),
        }
    }

    fn on_command_packet(&mut self, ptype: PacketType, header: &Header, payload: &[u8]) {
        let qi = if ptype == PacketType::Command { 0 } else { 1 };
        let id = header.packet_id;
        match self.queues[qi].classify(id) {
            Slot::Old => {
                self.send_ack(ptype, id);
                return;
            }
            Slot::TooFar => return,
            Slot::Next | Slot::Ahead => {}
        }
        if header.has(FLAG_UNENCRYPTED) {
            return;
        }
        let (_, generation) = self.queues[qi].window.locate(id);
        let mut data = payload.to_vec();
        if !self.decrypt(header, ptype, generation, &mut data) {
            return;
        }
        self.mark_received();
        self.send_ack(ptype, id);
        if self.queues[qi].has_pending(id) {
            return;
        }
        for result in self.queues[qi].insert(id, header.flags(), data) {
            match result {
                Ok(bytes) => self.on_command(&bytes),
                Err(e) => self.log(format!("Dropped a malformed command: {e}")),
            }
            if self.finished.is_some() {
                break;
            }
        }
    }

    fn on_ack(&mut self, ptype: PacketType, header: &Header, payload: &[u8]) {
        let idx = ptype.index();
        let (_, generation) = self.in_windows[idx].locate(header.packet_id);
        let mut data = payload.to_vec();
        if !self.decrypt(header, ptype, generation, &mut data) {
            return;
        }
        self.in_windows[idx].advance_past(header.packet_id, generation);
        self.mark_received();
        if data.len() < 2 {
            return;
        }
        let acked = u16::from_be_bytes([data[0], data[1]]);
        let target = if ptype == PacketType::Ack { PacketType::Command } else { PacketType::CommandLow };
        if let Some(pos) = self.resend.iter().position(|p| p.ptype == target && p.id == acked) {
            let pending = self.resend.remove(pos);
            if pending.tries == 0 {
                self.rtt_sample(pending.last_sent.elapsed().as_secs_f32() * 1000.0);
            }
        }
        if self.phase == Phase::Disconnecting && self.disconnect_packet == Some(acked) {
            self.fail("disconnected");
        }
    }

    fn rtt_sample(&mut self, ms: f32) {
        match self.srtt_ms {
            None => {
                self.srtt_ms = Some(ms);
                self.rttvar_ms = ms / 2.0;
            }
            Some(srtt) => {
                self.rttvar_ms = 0.75 * self.rttvar_ms + 0.25 * (srtt - ms).abs();
                self.srtt_ms = Some(0.875 * srtt + 0.125 * ms);
            }
        }
        let srtt = self.srtt_ms.unwrap_or(ms);
        self.rto_ms = (srtt + (4.0 * self.rttvar_ms).max(50.0)).clamp(MIN_RTO_MS, MAX_RTO_MS);
    }

    fn on_ping(&mut self, header: &Header) {
        let Some(keys) = &self.keys else {
            return;
        };
        if !header.has(FLAG_UNENCRYPTED) || header.mac != keys.shared_mac {
            return;
        }
        self.mark_received();
        self.send_packet(PacketType::Pong, 0, &header.packet_id.to_be_bytes());
    }

    fn on_pong(&mut self, header: &Header, payload: &[u8]) {
        let Some(keys) = &self.keys else {
            return;
        };
        if !header.has(FLAG_UNENCRYPTED) || header.mac != keys.shared_mac || payload.len() < 2 {
            return;
        }
        self.mark_received();
        let id = u16::from_be_bytes([payload[0], payload[1]]);
        if let Some((sent_id, at)) = self.last_ping {
            if sent_id == id {
                let ms = at.elapsed().as_secs_f32() * 1000.0;
                self.stats.add_ping(ms);
                self.rtt_sample(ms);
                self.last_ping = None;
            }
        }
    }

    fn on_voice(&mut self, ptype: PacketType, header: &Header, payload: &[u8]) {
        if self.phase != Phase::Connected {
            return;
        }
        let idx = ptype.index();
        let (in_window, generation) = self.in_windows[idx].locate(header.packet_id);
        let mut data = payload.to_vec();
        if !self.decrypt(header, ptype, generation, &mut data) {
            self.voice_errors += 1;
            if self.voice_errors == 1 || self.voice_errors % 500 == 0 {
                self.log(format!("Dropped {} undecryptable voice packet(s)", self.voice_errors));
            }
            return;
        }
        if in_window {
            self.in_windows[idx].advance_past(header.packet_id, generation);
        }
        self.mark_received();
        let Some(voice) = parse_s2c_voice(&data) else {
            return;
        };
        let now = Instant::now();
        if is_end_of_stream(voice.data) {
            self.voice_ended.insert(voice.client_id, (voice.voice_id, now));
            if self.talking.remove(&voice.client_id).is_some() {
                self.set_talking(voice.client_id, false, false);
            }
        } else {
            let late = self
                .voice_ended
                .get(&voice.client_id)
                .is_some_and(|(marker, at)| now.duration_since(*at) < LATE_VOICE && is_older(voice.voice_id, *marker));
            if !late {
                self.voice_ended.remove(&voice.client_id);
                self.talking.insert(voice.client_id, now);
                self.set_talking(voice.client_id, true, ptype == PacketType::VoiceWhisper);
            }
        }
        if let Some(sink) = &mut self.voice {
            sink(VoicePacket {
                client_id: voice.client_id,
                voice_id: voice.voice_id,
                codec: voice.codec,
                data: voice.data,
                whisper: ptype == PacketType::VoiceWhisper,
            });
        }
    }

    fn set_talking(&mut self, client_id: u16, talking: bool, whisper: bool) {
        if self.book.set_talking(client_id, talking, whisper) {
            self.emit(Event::Talking { client_id, talking, whisper: talking && whisper });
        }
    }

    fn on_command(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        if self.opts.log_commands {
            let shown: String = text.chars().take(600).collect();
            self.log(format!("< {shown}"));
        }
        let cmd = Command::parse(&text);
        match cmd.name.as_str() {
            "initivexpand2" => self.on_initivexpand2(&cmd),
            "initivexpand" => self.fail("this server uses the pre-3.1 handshake, which is not supported"),
            "initserver" => self.on_initserver(&cmd),
            "error" => self.on_error(&cmd),
            "channellist" | "notifychannelcreated" | "notifychanneledited" | "notifychannelmoved" => {
                self.book.upsert_channels(&cmd);
                self.view_dirty = true;
            }
            "channellistfinished" => {
                self.send_tracked(Command::new("channelsubscribeall"));
                self.view_dirty = true;
            }
            "notifychanneldeleted" => {
                self.book.remove_channels(&cmd);
                self.view_dirty = true;
            }
            "notifyserveredited" | "notifyserverupdated" => {
                self.book.server.apply(&cmd);
                self.view_dirty = true;
            }
            "notifycliententerview" => {
                let announce =
                    cmd.num::<u8>("reasonid") != Some(2) && Instant::now() >= self.announce_after;
                for client in self.book.clients_entered(&cmd) {
                    if client.id == self.client_id {
                        self.shared.own_channel.store(client.channel, Ordering::Relaxed);
                    } else if announce {
                        self.emit(Event::ClientEntered { client });
                    }
                }
                self.view_dirty = true;
            }
            "notifyclientleftview" => self.on_client_left(&cmd),
            "notifyclientmoved" => {
                for (client, from, to) in self.book.clients_moved(&cmd) {
                    self.talking.remove(&client.id);
                    if client.id == self.client_id {
                        self.shared.own_channel.store(to, Ordering::Relaxed);
                    }
                    self.emit(Event::ClientMoved { client, from, to });
                }
                self.view_dirty = true;
            }
            "notifyclientupdated" => {
                self.book.clients_updated(&cmd);
                self.view_dirty = true;
            }
            "notifytextmessage" => {
                let target = match cmd.num::<u8>("targetmode") {
                    Some(1) => TextTarget::Client(cmd.num("target").unwrap_or(0)),
                    Some(3) => TextTarget::Server,
                    _ => TextTarget::Channel,
                };
                self.emit(Event::TextMessage {
                    target,
                    from_id: cmd.num("invokerid").unwrap_or(0),
                    from_name: cmd.get("invokername").unwrap_or("").to_string(),
                    text: cmd.get("msg").unwrap_or("").to_string(),
                });
            }
            "notifyclientpoke" => self.emit(Event::Poke {
                from_name: cmd.get("invokername").unwrap_or("").to_string(),
                text: cmd.get("msg").unwrap_or("").to_string(),
            }),
            "notifyservergrouplist" | "notifychannelgrouplist" => {
                self.book.set_groups(&cmd, cmd.name == "notifyservergrouplist");
                self.emit(Event::Groups {
                    server_groups: self.book.regular_groups(true),
                    channel_groups: self.book.regular_groups(false),
                });
                self.view_dirty = true;
            }
            "notifyservergroupclientadded" | "notifyservergroupclientdeleted" => {
                self.book.group_member(&cmd, cmd.name == "notifyservergroupclientadded");
                self.view_dirty = true;
            }
            "notifyclientchannelgroupchanged" => {
                self.book.channel_group_changed(&cmd);
                self.view_dirty = true;
            }
            "notifystartdownload" => self.on_start_download(&cmd),
            "notifystatusfiletransfer" => self.on_transfer_status(&cmd),
            "notifyconnectioninforequest" => {
                let info = self.stats.connection_info();
                self.send_command(&info);
            }
            _ => {}
        }
    }

    fn on_initivexpand2(&mut self, cmd: &Command) {
        if self.keys.is_some() || self.phase != Phase::Init(4) {
            return;
        }
        self.init_pending = None;
        if cmd.get("ot") != Some("1") {
            self.fail("the server did not offer the 3.1+ handshake (ot=1)");
            return;
        }
        let field = |k: &str| cmd.get(k).unwrap_or("");
        let hello = match ServerHello::from_base64(field("l"), field("beta"), field("omega"), field("proof")) {
            Ok(h) => h,
            Err(e) => {
                self.fail(format!("invalid server handshake: {e}"));
                return;
            }
        };
        let outcome = match handshake::complete_handshake(&self.opts.identity, &self.alpha, &hello) {
            Ok(o) => o,
            Err(e) => {
                self.fail(e.to_string());
                return;
            }
        };
        let now_unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Some(problem) = outcome.license.validity_problem(now_unix) {
            self.log(format!("Warning: {problem}"));
        }
        self.book.server.uid = outcome.server_uid.clone();
        self.log(format!(
            "Server identity {} (license issued to {})",
            outcome.server_uid,
            outcome.license.issuer().unwrap_or("unknown")
        ));
        self.send_command(&Command::new("clientek").arg("ek", &outcome.ek).arg("proof", &outcome.proof));
        self.keys = Some(outcome.keys);
        self.phase = Phase::WaitInitServer;
        self.send_clientinit();
    }

    fn send_clientinit(&mut self) {
        let uid = self.opts.identity.uid();
        let hw = |salt: &str| -> String {
            let mut hasher = Sha1::new();
            hasher.update(salt.as_bytes());
            hasher.update(uid.as_bytes());
            hasher.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
        };
        let mut cmd = Command::new("clientinit")
            .arg("client_nickname", &self.opts.nickname)
            .arg("client_version", CLIENT_VERSION)
            .arg("client_platform", CLIENT_PLATFORM)
            .arg("client_input_hardware", 1)
            .arg("client_output_hardware", 1)
            .arg("client_default_channel", &self.opts.default_channel)
            .arg("client_default_channel_password", hash_password(&self.opts.default_channel_password))
            .arg("client_server_password", hash_password(&self.opts.server_password))
            .arg("client_meta_data", "")
            .arg("client_version_sign", CLIENT_VERSION_SIGN)
            .arg("client_key_offset", self.opts.identity.key_offset)
            .arg("client_nickname_phonetic", &self.opts.identity.phonetic_nickname)
            .arg("client_default_token", "")
            .arg("hwid", format!("{},{}", hw("phishspeak-a"), hw("phishspeak-b")));
        if self.opts.input_muted {
            cmd.push("client_input_muted", 1);
        }
        if self.opts.output_muted {
            cmd.push("client_output_muted", 1);
        }
        self.send_command(&cmd);
    }

    fn on_initserver(&mut self, cmd: &Command) {
        if self.phase != Phase::WaitInitServer {
            return;
        }
        let Some(client_id) = cmd.num::<u16>("aclid") else {
            self.fail("the server did not assign a client id");
            return;
        };
        self.client_id = client_id;
        self.book.own_id = client_id;
        let uid = std::mem::take(&mut self.book.server.uid);
        self.book.server.apply(cmd);
        self.book.server.uid = uid;
        if let Some(name) = cmd.get("acn") {
            self.opts.nickname = name.to_string();
        }
        self.resend.retain(|p| !(p.ptype == PacketType::Command && p.id <= 2));
        self.phase = Phase::Connected;
        self.next_ping = Instant::now();
        self.announce_after = Instant::now() + ANNOUNCE_DELAY;
        self.shared.client_id.store(client_id, Ordering::Relaxed);
        self.shared.connected.store(true, Ordering::Relaxed);
        self.emit(Event::State(ConnectionState::Connected));
        self.emit(Event::Connected { client_id, server: self.book.server.clone() });
        self.view_dirty = true;
    }

    fn on_error(&mut self, cmd: &Command) {
        let id = cmd.num::<u32>("id").unwrap_or(0);
        let message = cmd.get("msg").unwrap_or("").to_string();
        let extra = cmd.get("extra_msg").unwrap_or("").to_string();
        if self.phase == Phase::Connected || self.phase == Phase::Disconnecting {
            if self.icon_error(cmd, id, &message) {
                return;
            }
            if id != 0 {
                self.emit(Event::ServerError { id, message, extra });
            }
            return;
        }
        if id == 0 {
            return;
        }
        if id == ERROR_IDENTITY_LEVEL {
            if let Ok(level) = extra.trim().parse::<u8>() {
                self.emit(Event::SecurityLevelRequired(level));
                self.fail(format!("the server requires identity security level {level}"));
                return;
            }
        }
        let detail = if extra.is_empty() { message } else { format!("{message} ({extra})") };
        self.fail(format!("the server refused the connection: {detail}"));
    }

    fn on_client_left(&mut self, cmd: &Command) {
        let reason_id = cmd.num::<u8>("reasonid").unwrap_or(0);
        let reason_msg = cmd.get("reasonmsg").unwrap_or("").to_string();
        let invoker = cmd.get("invokername").unwrap_or("").to_string();
        let describe = |what: &str| -> String {
            let mut s = what.to_string();
            if !invoker.is_empty() {
                s.push_str(&format!(" by {invoker}"));
            }
            if !reason_msg.is_empty() {
                s.push_str(&format!(" ({reason_msg})"));
            }
            s
        };
        let reason = match reason_id {
            3 => describe("connection lost"),
            4 => describe("kicked from the channel"),
            5 => describe("kicked from the server"),
            6 => describe("banned from the server"),
            7 => describe("server stopped"),
            11 => describe("server is shutting down"),
            8 => describe("left"),
            _ => describe("left view"),
        };
        for client in self.book.clients_left(cmd) {
            self.talking.remove(&client.id);
            self.voice_ended.remove(&client.id);
            if client.id == self.client_id {
                let text = if self.phase == Phase::Disconnecting { "disconnected".to_string() } else { reason.clone() };
                self.fail(text);
            } else if reason_id != 2 {
                self.emit(Event::ClientLeft { client, reason: reason.clone() });
            }
        }
        self.view_dirty = true;
    }

    fn queue_icon(&mut self, id: u32) {
        if id == 0 || is_standard_icon(id) || self.icon_pending.contains(&id) {
            return;
        }
        if self.phase != Phase::Connected {
            self.emit(Event::Icon { id, data: Err("not connected".to_string()) });
            return;
        }
        if self.icons_off_until.is_some_and(|until| Instant::now() < until) {
            self.emit(Event::Icon { id, data: Err(filetransfer::UNREACHABLE.to_string()) });
            return;
        }
        if self.icon_pending.len() >= ICON_QUEUE_LIMIT {
            self.emit(Event::Icon { id, data: Err("too many icons are waiting".to_string()) });
            return;
        }
        self.icon_pending.insert(id);
        self.icon_queue.push_back(id);
    }

    fn note_reach(&mut self, data: &Result<Vec<u8>, String>, now: Instant) {
        let missed = matches!(data, Err(reason) if reason.starts_with(filetransfer::UNREACHABLE));
        if !missed {
            self.icon_misses = 0;
            return;
        }
        self.icon_misses += 1;
        if self.icon_misses < ICON_MISS_LIMIT {
            return;
        }
        self.icon_misses = 0;
        self.icons_off_until = Some(now + ICON_GIVE_UP_FOR);
        self.log("The server's file port cannot be reached, so its icons are not fetched for a while");
        for id in std::mem::take(&mut self.icon_queue) {
            self.icon_pending.remove(&id);
            self.emit(Event::Icon { id, data: Err(filetransfer::UNREACHABLE.to_string()) });
        }
    }

    fn finish_icon(&mut self, id: u32, data: Result<Vec<u8>, String>) {
        self.icon_active = None;
        self.icon_job = None;
        self.icon_pending.remove(&id);
        self.emit(Event::Icon { id, data });
    }

    fn drive_icons(&mut self, now: Instant) {
        if let Some((id, asked)) = self.icon_active.as_ref().map(|active| (active.icon, active.asked)) {
            let done = match &self.icon_job {
                Some(job) => match job.try_recv() {
                    Ok(data) => Some(data),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => Some(Err("the download stopped".to_string())),
                },
                None if now.duration_since(asked) > ICON_ANSWER_TIMEOUT => {
                    Some(Err("the server did not answer".to_string()))
                }
                None => None,
            };
            if let Some(data) = done {
                self.note_reach(&data, now);
                self.finish_icon(id, data);
            }
        }
        if self.phase != Phase::Connected || self.icon_active.is_some() || now < self.next_icon_at {
            return;
        }
        let Some(id) = self.icon_queue.pop_front() else {
            return;
        };
        let transfer = self.next_transfer;
        self.next_transfer = self.next_transfer.checked_add(1).unwrap_or(1);
        self.send_tracked(filetransfer::init_download(transfer, &filetransfer::icon_path(id)));
        self.icon_active = Some(IconTransfer { transfer, icon: id, asked: now, return_code: self.return_code });
        self.next_icon_at = now + ICON_GAP;
    }

    fn on_start_download(&mut self, cmd: &Command) {
        let Some(active) = &self.icon_active else {
            return;
        };
        if cmd.num::<u16>("clientftfid") != Some(active.transfer) || self.icon_job.is_some() {
            return;
        }
        let id = active.icon;
        let Some(start) = filetransfer::parse_start(cmd) else {
            self.finish_icon(id, Err("the server's answer could not be read".to_string()));
            return;
        };
        if start.size > ICON_SIZE_LIMIT {
            self.send_tracked(filetransfer::stop(start.server_transfer));
            self.finish_icon(
                id,
                Err(format!("the file is {} bytes, more than the {ICON_SIZE_LIMIT} allowed", start.size)),
            );
            return;
        }
        let peer = match self.socket.peer_addr() {
            Ok(peer) => peer,
            Err(e) => {
                self.finish_icon(id, Err(format!("the server's address is not known: {e}")));
                return;
            }
        };
        let addr = SocketAddr::new(peer.ip(), self.opts.filetransfer_port.unwrap_or(start.port));
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("ps-client-ft".into()).spawn(move || {
            let _ = tx.send(filetransfer::download(addr, &start.key, start.size, ICON_SIZE_LIMIT, ICON_TRANSFER_TIMEOUT));
        });
        match spawned {
            Ok(_) => self.icon_job = Some(rx),
            Err(e) => self.finish_icon(id, Err(format!("cannot start the download: {e}"))),
        }
    }

    fn on_transfer_status(&mut self, cmd: &Command) {
        let (Some(status), Some(active)) = (filetransfer::parse_status(cmd), &self.icon_active) else {
            return;
        };
        if status.transfer != active.transfer || self.icon_job.is_some() {
            return;
        }
        let id = active.icon;
        let reason = if status.message.is_empty() { format!("status {}", status.code) } else { status.message };
        self.finish_icon(id, Err(reason));
    }

    fn icon_error(&mut self, cmd: &Command, id: u32, message: &str) -> bool {
        if id == ERROR_FLOODING {
            self.next_icon_at = Instant::now() + ICON_FLOOD_PAUSE;
        }
        let Some(active) = &self.icon_active else {
            return false;
        };
        let code = cmd.num::<u32>("return_code");
        let ours = code == Some(active.return_code);
        let maybe_ours = id == ERROR_FLOODING && code.is_none();
        if !ours && !maybe_ours {
            return false;
        }
        if id == 0 || self.icon_job.is_some() {
            return ours;
        }
        let icon = active.icon;
        if id == ERROR_FLOODING {
            self.icon_active = None;
            self.icon_queue.push_front(icon);
            self.log("The server asked for fewer requests; icons will continue in a moment");
        } else {
            self.finish_icon(icon, Err(message.to_string()));
        }
        ours
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.finished.is_some() {
            return;
        }
        if let Some(deadline) = self.disconnect_deadline {
            if now >= deadline {
                self.fail("disconnected");
                return;
            }
        }
        if !matches!(self.phase, Phase::Connected | Phase::Disconnecting)
            && now.duration_since(self.started) > CONNECT_TIMEOUT
        {
            if self.received_any {
                self.fail("the handshake with the server timed out");
            } else {
                self.fail(format!(
                    "no response from {}:{} (is a TeamSpeak 3 server running there?)",
                    self.opts.host, self.opts.port
                ));
            }
            return;
        }
        if self.phase == Phase::Connected && now.duration_since(self.last_rx) > PACKET_TIMEOUT {
            self.fail("connection lost (the server stopped responding)");
            return;
        }

        let rto = self.rto_ms;
        let mut timed_out = false;
        let mut to_send: Vec<(PacketType, Vec<u8>)> = Vec::new();
        for p in self.init_pending.iter_mut().chain(self.resend.iter_mut()) {
            if now.duration_since(p.first_sent) > PACKET_TIMEOUT {
                timed_out = true;
                break;
            }
            let wait = (rto * (1u32 << p.tries.min(3)) as f32).min(MAX_RTO_MS);
            if now.duration_since(p.last_sent).as_secs_f32() * 1000.0 >= wait {
                p.last_sent = now;
                p.tries += 1;
                to_send.push((p.ptype, p.raw.clone()));
            }
        }
        if timed_out {
            self.fail("connection lost (a packet was never acknowledged)");
            return;
        }
        for (ptype, raw) in to_send {
            self.stats.resent += 1;
            self.send_raw(ptype, &raw);
        }

        if self.phase == Phase::Connected && now >= self.next_ping {
            self.next_ping = now + PING_INTERVAL;
            let (id, _) = self.send_packet(PacketType::Ping, 0, &[]);
            self.last_ping = Some((id, now));
        }

        self.drive_icons(now);

        let stale: Vec<u16> = self
            .talking
            .iter()
            .filter(|(_, at)| now.duration_since(**at) > TALK_TIMEOUT)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.talking.remove(&id);
            self.set_talking(id, false, false);
        }

        if self.view_dirty && now.duration_since(self.last_view) >= VIEW_INTERVAL {
            self.view_dirty = false;
            self.last_view = now;
            self.shared.own_channel.store(self.book.own_channel(), Ordering::Relaxed);
            self.emit(Event::View(self.book.view()));
        }

        if self.phase == Phase::Connected && now.duration_since(self.last_stats) >= STATS_INTERVAL {
            self.last_stats = now;
            self.emit(Event::Stats(LinkStats {
                ping_ms: self.stats.ping(),
                ping_deviation_ms: self.stats.ping_deviation(),
                packets_resent: self.stats.resent,
                voice_packets_in: self.stats.voice_in(),
                voice_packets_out: self.stats.voice_out(),
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_packet_older_than_the_end_packet_is_recognised() {
        assert!(is_older(5, 6));
        assert!(!is_older(6, 6));
        assert!(!is_older(7, 6));
        assert!(is_older(65535, 0));
        assert!(!is_older(0, 65535));
        assert!(is_older(65000, 100));
    }
}
