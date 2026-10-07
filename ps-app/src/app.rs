use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ps_client::spacer::{parse_spacer, SpacerAlign, SpacerLine};
use ps_client::{
    ClientHandle, ConnectOptions, TextTarget, VoiceSink, WhisperTarget, CODEC_OPUS_VOICE, DEFAULT_PORT,
};
use ps_identity::Identity;
use ps_voice::codec::CODEC_CELT_MONO;
use ps_voice::{AudioEngine, Cue, DeviceInfo, FrameSink, TxMode};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

use crate::bookmarks::{initials, Bookmark, Bookmarks};
use crate::hotkeys::chord_name;
use crate::icons::{IconStore, Lookup};
use crate::keywatch::KeyWatcher;
use crate::platform;
use crate::instance::{self, Wish};
use crate::links::{self, Link};
use crate::scale::{ScaleWatch, Step};
use crate::speakers::{self, Member, Room, Roster};
use crate::session::{
    self, build_rows_folded, connect_failure, mic_move, next_view, ChannelIcon, ChatKind, ChatLine, ConnectRequest,
    DialogField, FoldMode, MicMove, Outcome, Peer, RowData, RowKind, Session,
};
use crate::settings::{self, Settings, Voice, MAX_REMEMBERED_VOICES};
use crate::whisper::{route, Route, WhisperKeys};
use crate::{
    BookmarkRow, ChatRow, Icons, IdentityRow, PhishSpeakApp, PickRow, ServerTile, SettingsWindow, SpeakerRow,
    SpeakersWindow, TreeRow, WhisperKeyRow,
};

const SPEAKERS_TITLE: &str = "PhishSpeak speaking";
const MAIN_TITLE: &str = "PhishSpeak";

mod shortcuts;

const SETTINGS_SAVE_DELAY: Duration = Duration::from_secs(2);
const METER_FLOOR_DB: f32 = -70.0;
const SILENCE_DB: f32 = -95.9;
const SILENCE_HINT_AFTER: Duration = Duration::from_secs(4);
const DEFAULT_NICKNAME: &str = "PhishSpeakUser";
const HEADER_TILES: usize = 3;

pub fn level_position(db: f32) -> f32 {
    ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

fn string_model(items: Vec<String>) -> ModelRc<SharedString> {
    let shared: Vec<SharedString> = items.into_iter().map(SharedString::from).collect();
    ModelRc::new(VecModel::from(shared))
}

fn sync_rows<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    let (old, new) = (model.row_count(), rows.len());
    let same = |at: usize, row: &T| model.row_data(at).as_ref() == Some(row);
    let mut head = 0;
    while head < old.min(new) && same(head, &rows[head]) {
        head += 1;
    }
    let mut tail = 0;
    while tail < old.min(new) - head && same(old - 1 - tail, &rows[new - 1 - tail]) {
        tail += 1;
    }
    let (gone, added) = (old - head - tail, new - head - tail);
    let kept = gone.min(added);
    for (offset, row) in rows.into_iter().skip(head).take(added).enumerate() {
        if offset >= kept {
            model.insert(head + offset, row);
        } else if !same(head + offset, &row) {
            model.set_row_data(head + offset, row);
        }
    }
    for _ in kept..gone {
        model.remove(head + added);
    }
}

fn tree_row(row: &RowData) -> TreeRow {
    TreeRow {
        kind: match row.kind {
            RowKind::SpacerText => 0,
            RowKind::SpacerLine => 1,
            RowKind::Gap => 2,
            RowKind::Channel => 3,
            RowKind::Person => 4,
        },
        id: row.id as i32,
        depth: row.depth.min(12) as i32,
        text: row.text.as_str().into(),
        align: match row.align {
            SpacerAlign::Left => 0,
            SpacerAlign::Center => 1,
            SpacerAlign::Right => 2,
            SpacerAlign::Repeat => 3,
        },
        line: match row.line {
            SpacerLine::Solid => 0,
            SpacerLine::Dashed => 1,
            SpacerLine::Dotted => 2,
        },
        icon: match row.icon {
            ChannelIcon::Speaker => 0,
            ChannelIcon::Lock => 1,
            ChannelIcon::Music => 2,
        },
        count: row.count as i32,
        current: row.current,
        talking: row.talking,
        me: row.me,
        mic_muted: row.mic_muted,
        sound_muted: row.sound_muted,
        away: row.away,
        tag: row.tag.as_str().into(),
        whispering: row.whispering,
        commander: row.commander,
        foldable: row.foldable,
        folded: row.folded,
        badges: 0,
        badge_tint: 0,
        badge_a: slint::Image::default(),
        badge_b: slint::Image::default(),
        badge_c: slint::Image::default(),
        badge_d: slint::Image::default(),
    }
}

fn set_badge(row: &mut TreeRow, picture: slint::Image, tinted: bool) {
    let slot = row.badges;
    match slot {
        0 => row.badge_a = picture,
        1 => row.badge_b = picture,
        2 => row.badge_c = picture,
        3 => row.badge_d = picture,
        _ => return,
    }
    if tinted {
        row.badge_tint |= 1 << slot;
    }
    row.badges += 1;
}

fn chat_row(line: &ChatLine) -> ChatRow {
    ChatRow {
        time: line.time.as_str().into(),
        kind: match line.kind {
            ChatKind::System => 0,
            ChatKind::Message => 1,
            ChatKind::Mine => 2,
            ChatKind::Error => 3,
        },
        name: line.name.as_str().into(),
        text: line.text.as_str().into(),
    }
}

fn same_address(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

pub fn forget_links(scheme: &str) {
    let mut settings = Settings::load();
    if !settings.links.on {
        return;
    }
    if links::release(&mut LinkRegistry { scheme: scheme.to_string() }, &mut settings.links) {
        let _ = settings.save();
    }
}

fn own_program() -> Option<String> {
    std::env::current_exe().ok().and_then(|path| path.to_str().map(str::to_string))
}

struct LinkRegistry {
    scheme: String,
}

impl links::Handlers for LinkRegistry {
    fn current(&self) -> Option<String> {
        platform::link_handler(&self.scheme)
    }

    fn set(&mut self, command: &str) -> bool {
        platform::set_link_handler(&self.scheme, command)
    }

    fn clear(&mut self) -> bool {
        platform::clear_link_handler(&self.scheme)
    }
}

pub struct Windows {
    pub main: PhishSpeakApp,
    pub settings: SettingsWindow,
    pub speakers: SpeakersWindow,
}

#[derive(Debug, Clone, Default)]
pub struct StartRequest {
    pub target: String,
    pub nickname: String,
    pub channel: String,
}

struct LoadedIdentity {
    path: PathBuf,
    identity: Identity,
}

struct LevelJob {
    uid: String,
    target: u8,
    stop: Arc<AtomicBool>,
    result: Receiver<(u64, bool)>,
}

#[derive(Default)]
struct Dirty {
    sessions: bool,
    tree: bool,
    chat: bool,
    bookmarks: bool,
    identities: bool,
    shortcuts: bool,
    speakers: bool,
}

impl Dirty {
    fn everything() -> Self {
        Self { sessions: true, tree: true, chat: true, bookmarks: true, identities: true, shortcuts: true, speakers: true }
    }
}

pub struct App {
    main: Weak<PhishSpeakApp>,
    settings_window: Weak<SettingsWindow>,
    speakers_window: Weak<SpeakersWindow>,
    speaker_rows: Rc<VecModel<SpeakerRow>>,
    speakers_ticks: u32,
    speakers_empty: bool,
    speakers_look: Option<(u8, bool)>,
    roster: Roster,
    engine: AudioEngine,
    watcher: KeyWatcher,
    pub settings: Settings,
    whisper_keys: WhisperKeys,
    lanes: Arc<Mutex<Vec<Option<WhisperTarget>>>>,
    lane_names: Vec<String>,
    lane_notes: Vec<String>,
    lanes_stale: bool,
    reply_live: Option<(u16, u16, String)>,
    unheard: Option<(u8, Instant)>,
    whisper_air: Option<u8>,
    last_whisper_lane: u8,
    allow_whispers: Arc<AtomicBool>,
    capture: Option<shortcuts::CaptureTarget>,
    editor: Option<shortcuts::Editor>,
    shortcut_note: (i32, String),
    talk_rows: Rc<VecModel<SharedString>>,
    whisper_rows: Rc<VecModel<WhisperKeyRow>>,
    pick_rows: Rc<VecModel<PickRow>>,
    group_rows: Rc<VecModel<SharedString>>,
    tree_shown: Option<u16>,
    start_rows: Rc<VecModel<SharedString>>,
    bm_options: Vec<(String, String, u64)>,
    bm_choice: (String, u64),
    save_at: Option<Instant>,
    bookmarks: Bookmarks,
    identities: Vec<LoadedIdentity>,
    sessions: Vec<Session>,
    next_id: u16,
    viewed: Option<u16>,
    mic_target: Option<u16>,
    own_talking: bool,
    mic_muted: bool,
    sound_muted: bool,
    tree: Rc<VecModel<TreeRow>>,
    chat: Rc<VecModel<ChatRow>>,
    chat_shown: Option<(u16, u64)>,
    inputs: Vec<DeviceInfo>,
    outputs: Vec<DeviceInfo>,
    level_jobs: Vec<LevelJob>,
    prompt: Option<(u16, u64)>,
    person: Option<(u16, u16)>,
    key_prompt: Option<u16>,
    channel_sheet: Option<(u16, u64)>,
    scales: [ScaleWatch; 3],
    instance: Option<instance::Listener>,
    pending_link: Option<Link>,
    link_scheme: String,
    dirty: Dirty,
    silent_since: Option<Instant>,
    silence_warned: bool,
    warned_codecs: Vec<u8>,
    shown_status: String,
    icons: IconStore,
    standard_icons: Vec<slint::Image>,
    shown_echo: String,
    shown_wide: bool,
    shown_devices: (String, String),
    trace: bool,
}

pub fn with_app(app: &Rc<RefCell<App>>, f: impl FnOnce(&mut App, &Windows)) {
    let Ok(mut state) = app.try_borrow_mut() else {
        return;
    };
    let (Some(main), Some(settings), Some(speakers)) =
        (state.main.upgrade(), state.settings_window.upgrade(), state.speakers_window.upgrade())
    else {
        return;
    };
    let windows = Windows { main, settings, speakers };
    f(&mut state, &windows);
    state.refresh(&windows);
}

impl App {
    pub fn new(
        main: &PhishSpeakApp,
        settings_window: &SettingsWindow,
        speakers_window: &SpeakersWindow,
        settings: Settings,
    ) -> Self {
        let engine = AudioEngine::start();
        if !settings.input_device.is_empty() {
            engine.set_input_device(Some(settings.input_device.clone()));
        }
        if !settings.output_device.is_empty() {
            engine.set_output_device(Some(settings.output_device.clone()));
        }
        let watcher = KeyWatcher::start(engine.shared().clone());
        let allow_whispers = Arc::new(AtomicBool::new(settings.allow_whispers));
        Self {
            main: main.as_weak(),
            settings_window: settings_window.as_weak(),
            speakers_window: speakers_window.as_weak(),
            speaker_rows: Rc::new(VecModel::default()),
            speakers_ticks: 0,
            speakers_empty: true,
            speakers_look: None,
            roster: Roster::default(),
            engine,
            watcher,
            settings,
            whisper_keys: WhisperKeys::load(),
            lanes: Arc::new(Mutex::new(Vec::new())),
            lane_names: Vec::new(),
            lane_notes: Vec::new(),
            lanes_stale: false,
            reply_live: None,
            unheard: None,
            whisper_air: None,
            last_whisper_lane: 0,
            allow_whispers,
            capture: None,
            editor: None,
            shortcut_note: (0, String::new()),
            talk_rows: Rc::new(VecModel::default()),
            whisper_rows: Rc::new(VecModel::default()),
            pick_rows: Rc::new(VecModel::default()),
            group_rows: Rc::new(VecModel::default()),
            tree_shown: None,
            start_rows: Rc::new(VecModel::default()),
            bm_options: Vec::new(),
            bm_choice: (String::new(), 0),
            save_at: None,
            bookmarks: Bookmarks::load(),
            identities: Vec::new(),
            sessions: Vec::new(),
            next_id: 1,
            viewed: None,
            mic_target: None,
            own_talking: false,
            mic_muted: false,
            sound_muted: false,
            tree: Rc::new(VecModel::default()),
            chat: Rc::new(VecModel::default()),
            chat_shown: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            level_jobs: Vec::new(),
            prompt: None,
            person: None,
            key_prompt: None,
            channel_sheet: None,
            scales: [ScaleWatch::default(), ScaleWatch::default(), ScaleWatch::default()],
            instance: None,
            pending_link: None,
            link_scheme: links::scheme(),
            dirty: Dirty::everything(),
            silent_since: None,
            silence_warned: false,
            warned_codecs: Vec::new(),
            shown_status: String::new(),
            icons: IconStore::new(settings::config_dir().join("cache").join("icons")),
            standard_icons: Vec::new(),
            shown_echo: String::new(),
            shown_wide: false,
            shown_devices: (String::new(), String::new()),
            trace: std::env::var_os("PHISHSPEAK_TRACE").is_some(),
        }
    }

    pub fn start(&mut self, w: &Windows, wishes: &[Wish]) {
        w.main.set_tree(ModelRc::from(self.tree.clone()));
        w.main.set_chat(ModelRc::from(self.chat.clone()));
        w.settings.set_talk_keys(ModelRc::from(self.talk_rows.clone()));
        w.settings.set_whisper_keys(ModelRc::from(self.whisper_rows.clone()));
        w.settings.set_editor_rows(ModelRc::from(self.pick_rows.clone()));
        w.settings.set_editor_groups(ModelRc::from(self.group_rows.clone()));
        w.settings.set_bm_channels(ModelRc::from(self.start_rows.clone()));
        w.settings.set_version(env!("CARGO_PKG_VERSION").into());
        w.speakers.set_rows(ModelRc::from(self.speaker_rows.clone()));
        self.apply_speakers(w);
        let drawn = w.main.global::<Icons>();
        self.standard_icons = vec![
            drawn.get_group_100(),
            drawn.get_group_200(),
            drawn.get_group_300(),
            drawn.get_group_500(),
            drawn.get_group_600(),
        ];
        w.settings.set_tx_mode(self.settings.tx_mode);
        w.settings.set_vad_threshold(self.settings.vad_threshold);
        w.settings.set_mic_gain(self.settings.mic_gain);
        w.settings.set_output_volume(self.settings.output_volume);
        w.settings.set_echo_cancel(self.settings.echo_cancel);
        w.settings.set_noise_suppression(self.settings.noise_suppression);
        w.settings.set_auto_gain(self.settings.auto_gain);
        w.settings.set_cue_volume(self.settings.cue_volume);
        let problems = self.load_identities();
        if self.identities.is_empty() {
            if let Err(problem) = self.create_identity() {
                w.main.set_notice(problem.into());
            }
        } else if let Some(problem) = problems.first() {
            w.settings.set_identity_note(problem.as_str().into());
        }
        self.refresh_devices(w);
        self.apply_audio(w);
        self.refresh_links(w);
        self.push_bindings();
        self.rebuild_lanes(true);
        self.save_at = None;
        self.dirty = Dirty::everything();
        let wanted: Vec<Bookmark> = self.bookmarks.items.iter().filter(|b| b.auto_connect).cloned().collect();
        for bookmark in wanted.iter().rev() {
            let mut request = self.bookmark_request(bookmark);
            request.quiet = true;
            self.begin(w, request);
        }
        for wish in wishes {
            self.grant(w, wish.clone());
        }
    }

    pub fn attach_instance(&mut self, listener: instance::Listener) {
        self.instance = Some(listener);
    }

    fn refresh_links(&mut self, w: &Windows) {
        let before = self.settings.links.clone();
        if let Some(program) = own_program() {
            links::refresh(&mut LinkRegistry { scheme: self.link_scheme.clone() }, &program, &mut self.settings.links);
        }
        if self.settings.links != before {
            self.mark_settings_dirty();
        }
        w.settings.set_links_on(self.settings.links.on);
    }

    pub fn links_changed(&mut self, w: &Windows) {
        let wanted = w.settings.get_links_on();
        let mut registry = LinkRegistry { scheme: self.link_scheme.clone() };
        let done = match own_program() {
            Some(program) if wanted => links::claim(&mut registry, &program, &mut self.settings.links),
            Some(_) => links::release(&mut registry, &mut self.settings.links),
            None => false,
        };
        w.settings.set_links_on(self.settings.links.on);
        w.settings.set_links_note(if done { "" } else { "Windows did not allow that change." }.into());
        self.mark_settings_dirty();
    }

    fn grant(&mut self, w: &Windows, wish: Wish) {
        match wish {
            Wish::Show => {}
            Wish::Link(text) => self.open_link(w, &text),
            Wish::Connect { target, nickname, channel } => {
                self.connect_from_start(w, &StartRequest { target, nickname, channel });
            }
        }
    }

    fn take_wishes(&mut self, w: &Windows) {
        let wishes: Vec<Wish> =
            self.instance.as_ref().map(|listener| listener.wishes.try_iter().collect()).unwrap_or_default();
        if wishes.is_empty() {
            return;
        }
        for wish in wishes {
            self.grant(w, wish);
        }
        platform::show_own_window(MAIN_TITLE);
    }

    pub fn open_link(&mut self, w: &Windows, text: &str) {
        let Ok(link) = links::parse(text, &self.link_scheme) else {
            w.main.set_notice("That link could not be read, so nothing was done with it.".into());
            return;
        };
        let open = self.sessions.iter().find(|s| same_address(&s.request.address, &link.address)).map(|s| s.id);
        if let Some(id) = open {
            self.view_server(w, id);
            return;
        }
        let known = self.bookmarks.find_address(&link.address).map(|index| self.bookmarks.items[index].clone());
        let mut request = match &known {
            Some(bookmark) => self.bookmark_request(bookmark),
            None => {
                let index = self.default_identity();
                ConnectRequest {
                    nickname: self.default_nickname(),
                    identity_uid: if index >= 0 { self.identities[index as usize].identity.uid() } else { String::new() },
                    ..ConnectRequest::default()
                }
            }
        };
        request.address = link.address.clone();
        request.password = link.password.clone();
        if !link.nickname.is_empty() {
            request.nickname = link.nickname.clone();
        }
        let mut link = link;
        if known.is_some() {
            link.bookmark.clear();
        }
        request.save_bookmark = !link.bookmark.is_empty();
        self.pending_link = Some(link);
        self.show_dialog(w, &request, None);
    }

    fn mark_settings_dirty(&mut self) {
        if self.save_at.is_none() {
            self.save_at = Some(Instant::now() + SETTINGS_SAVE_DELAY);
        }
    }

    fn session(&self, id: u16) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    fn session_mut(&mut self, id: u16) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    fn viewed_session(&self) -> Option<&Session> {
        self.viewed.and_then(|id| self.session(id))
    }

    fn identity_index(&self, uid: &str) -> Option<usize> {
        self.identities.iter().position(|l| l.identity.uid() == uid)
    }

    fn default_identity(&self) -> i32 {
        if self.identities.is_empty() {
            return -1;
        }
        self.identity_index(&self.settings.identity_uid).unwrap_or(0) as i32
    }

    fn default_nickname(&self) -> String {
        let saved = self.settings.nickname.trim();
        if !saved.is_empty() {
            return saved.to_string();
        }
        let index = self.default_identity();
        let from_identity =
            if index >= 0 { self.identities[index as usize].identity.nickname.trim().to_string() } else { String::new() };
        if from_identity.is_empty() {
            DEFAULT_NICKNAME.to_string()
        } else {
            from_identity
        }
    }

    fn identity_sources(&self) -> Vec<PathBuf> {
        let mut sources: Vec<PathBuf> = self
            .settings
            .identity_path
            .split('|')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect();
        sources.push(settings::identities_dir());
        sources
    }

    fn load_identities(&mut self) -> Vec<String> {
        let mut found: Vec<LoadedIdentity> = Vec::new();
        let mut problems = Vec::new();
        for source in self.identity_sources() {
            for (path, result) in ps_identity::list_identities(&source) {
                match result {
                    Ok(mut identity) => {
                        if identity.private_key.is_none() {
                            problems.push(format!("{} has no private key, so it was skipped.", path.display()));
                            continue;
                        }
                        let uid = identity.uid();
                        if let Some(offset) = self.settings.key_offsets.get(&uid) {
                            if *offset > identity.key_offset {
                                identity.key_offset = *offset;
                            }
                        }
                        if !found.iter().any(|l| l.identity.uid() == uid) {
                            found.push(LoadedIdentity { path, identity });
                        }
                    }
                    Err(e) => problems.push(format!("{} could not be read: {e}.", path.display())),
                }
            }
        }
        self.identities = found;
        self.dirty.identities = true;
        problems
    }

    fn create_identity(&mut self) -> Result<String, String> {
        let dir = settings::identities_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}.", dir.display()))?;
        let mut number = 1;
        let path = loop {
            let candidate = dir.join(format!("identity_{number}.ini"));
            if !candidate.exists() {
                break candidate;
            }
            number += 1;
        };
        let nickname = self.default_nickname();
        let identity = Identity::generate(&format!("PhishSpeak {number}"), &nickname);
        identity.save(&path).map_err(|e| format!("Could not save the new identity: {e}."))?;
        let uid = identity.uid();
        let name = identity.name.clone();
        self.identities.push(LoadedIdentity { path, identity });
        if self.settings.identity_uid.is_empty() {
            self.settings.identity_uid = uid;
            self.mark_settings_dirty();
        }
        self.dirty.identities = true;
        Ok(name)
    }

    pub fn new_identity(&mut self, w: &Windows) {
        let note = match self.create_identity() {
            Ok(name) => format!("Created {name}."),
            Err(problem) => problem,
        };
        w.settings.set_identity_note(note.into());
    }

    pub fn import_identity(&mut self, w: &Windows) {
        let typed = w.settings.get_import_path().trim().trim_matches('"').to_string();
        if typed.is_empty() {
            w.settings.set_identity_note("Type the path of an identity file or a folder of them.".into());
            return;
        }
        let path = PathBuf::from(&typed);
        let usable = ps_identity::list_identities(&path)
            .into_iter()
            .filter(|(_, result)| result.as_ref().is_ok_and(|identity| identity.private_key.is_some()))
            .count();
        if usable == 0 {
            w.settings.set_identity_note(
                "No usable TeamSpeak identity was found there. Export one from the TeamSpeak client and try again."
                    .into(),
            );
            return;
        }
        let already = self.settings.identity_path.split('|').any(|p| p.trim().eq_ignore_ascii_case(&typed));
        if !already {
            if !self.settings.identity_path.trim().is_empty() {
                self.settings.identity_path.push('|');
            }
            self.settings.identity_path.push_str(&typed);
            self.mark_settings_dirty();
        }
        let before = self.identities.len();
        self.load_identities();
        let added = self.identities.len().saturating_sub(before);
        let note = match added {
            0 => "Those identities were already in the list.".to_string(),
            1 => "Added 1 identity. The file stays where it is and is never changed.".to_string(),
            n => format!("Added {n} identities. The files stay where they are and are never changed."),
        };
        w.settings.set_identity_note(note.into());
        w.settings.set_import_path("".into());
    }

    fn refresh_devices(&mut self, w: &Windows) {
        self.inputs = ps_voice::list_input_devices();
        self.outputs = ps_voice::list_output_devices();
        let mut input_names = vec!["Default microphone".to_string()];
        input_names.extend(self.inputs.iter().map(|d| d.name.clone()));
        let mut output_names = vec!["Default speakers".to_string()];
        output_names.extend(self.outputs.iter().map(|d| d.name.clone()));
        w.settings.set_input_devices(string_model(input_names));
        w.settings.set_output_devices(string_model(output_names));
        let input_index =
            self.inputs.iter().position(|d| d.id == self.settings.input_device).map(|i| i + 1).unwrap_or(0);
        let output_index =
            self.outputs.iter().position(|d| d.id == self.settings.output_device).map(|i| i + 1).unwrap_or(0);
        w.settings.set_input_index(input_index as i32);
        w.settings.set_output_index(output_index as i32);
    }

    pub fn select_input(&mut self, w: &Windows) {
        let index = w.settings.get_input_index();
        let id = if index <= 0 { None } else { self.inputs.get(index as usize - 1).map(|d| d.id.clone()) };
        self.settings.input_device = id.clone().unwrap_or_default();
        self.engine.set_input_device(id);
        self.silent_since = None;
        self.mark_settings_dirty();
    }

    pub fn select_output(&mut self, w: &Windows) {
        let index = w.settings.get_output_index();
        let id = if index <= 0 { None } else { self.outputs.get(index as usize - 1).map(|d| d.id.clone()) };
        self.settings.output_device = id.clone().unwrap_or_default();
        self.engine.set_output_device(id);
        self.mark_settings_dirty();
    }

    pub fn apply_audio(&mut self, w: &Windows) {
        let tx_mode = w.settings.get_tx_mode().clamp(0, 2);
        let threshold = w.settings.get_vad_threshold().clamp(METER_FLOOR_DB, 0.0);
        let gain = w.settings.get_mic_gain().clamp(0.0, 300.0);
        let volume = w.settings.get_output_volume().clamp(0.0, 200.0);
        let shared = self.engine.shared();
        shared.set_tx_mode(TxMode::from_index(tx_mode as u8));
        shared.set_vad_threshold(threshold);
        shared.set_input_gain(gain / 100.0);
        shared.set_output_volume(volume / 100.0);
        let echo = w.settings.get_echo_cancel();
        shared.set_echo_cancel(echo);
        let denoise = w.settings.get_noise_suppression();
        let even = w.settings.get_auto_gain();
        let cues = w.settings.get_cue_volume().clamp(0.0, 100.0);
        shared.set_noise_suppression(denoise);
        shared.set_auto_gain(even);
        shared.set_cue_volume(cues / 100.0);
        self.engine.set_loopback(w.settings.get_mic_test());
        w.main.set_threshold_position(if tx_mode == 0 { level_position(threshold) } else { -1.0 });
        self.settings.tx_mode = tx_mode;
        self.settings.vad_threshold = threshold;
        self.settings.mic_gain = gain;
        self.settings.output_volume = volume;
        self.settings.echo_cancel = echo;
        self.settings.noise_suppression = denoise;
        self.settings.auto_gain = even;
        self.settings.cue_volume = cues;
        self.mark_settings_dirty();
    }

    pub fn open_settings(&mut self, w: &Windows, tab: i32) {
        w.settings.set_tab(tab.clamp(0, 6));
        self.dirty.shortcuts = true;
        self.refresh_devices(w);
        if w.settings.get_bm_index() < 0 && w.settings.get_bm_address().is_empty() {
            self.bookmark_new(w);
        }
        self.dirty.bookmarks = true;
        self.dirty.identities = true;
        let _ = w.settings.show();
    }

    pub fn stop_mic_test(&mut self, w: &Windows) {
        w.settings.set_mic_test(false);
        self.engine.set_loopback(false);
    }

    pub fn settings_hidden(&mut self, w: &Windows) {
        self.stop_mic_test(w);
        self.close_editor();
    }

    pub fn close_settings(&mut self, w: &Windows) {
        self.settings_hidden(w);
        let _ = w.settings.hide();
    }

    fn bookmark_rows(&self, skip_connected: bool) -> Vec<BookmarkRow> {
        self.bookmarks
            .items
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                !skip_connected || !self.sessions.iter().any(|s| same_address(&s.request.address, &b.address))
            })
            .map(|(index, b)| BookmarkRow {
                index: index as i32,
                initials: initials(&b.name).into(),
                name: b.name.as_str().into(),
                address: b.address.as_str().into(),
                nickname: b.nickname.as_str().into(),
                identity_index: self.identity_index(&b.identity_uid).map(|i| i as i32).unwrap_or(-1),
            })
            .collect()
    }

    fn save_bookmarks(&mut self, w: &Windows) {
        if let Err(e) = self.bookmarks.save() {
            w.main.set_notice(format!("Could not save your bookmarks: {e}.").into());
        }
        self.dirty.bookmarks = true;
    }

    fn enter_start_channel(&mut self, w: &Windows, id: u16, channel: u64, locked: bool, password: &str) {
        let Some(session) = self.session(id) else {
            return;
        };
        let Some(client) = &session.client else {
            return;
        };
        if !locked {
            client.join_channel(channel, "");
        } else if !password.is_empty() {
            client.join_channel(channel, password);
        } else if self.viewed == Some(id) && !w.main.get_prompt_open() {
            w.main.set_prompt_kind(0);
            w.main.set_prompt_name(session.channel_name(channel).into());
            w.main.set_prompt_password("".into());
            w.main.set_prompt_open(true);
            self.prompt = Some((id, channel));
            self.key_prompt = None;
        }
    }

    fn start_options(&self, address: &str) -> (Vec<(String, String, u64)>, bool) {
        let mut options = vec![("The channel the server puts me in".to_string(), String::new(), 0u64)];
        let live = self
            .sessions
            .iter()
            .filter(|s| s.is_connected() && same_address(&s.request.address, address))
            .find_map(|s| s.view.as_ref());
        if let Some(view) = live {
            for node in &view.channels {
                if parse_spacer(&node.channel.name, node.channel.parent).is_some() {
                    continue;
                }
                if let Some(path) = session::channel_path(view, node.channel.id) {
                    let indent = "    ".repeat(node.depth.min(8) as usize);
                    options.push((format!("{indent}{}", node.channel.name), path, node.channel.id));
                }
            }
        }
        (options, live.is_some())
    }

    fn publish_start_channels(&mut self, w: &Windows) {
        let address = w.settings.get_bm_address().trim().to_string();
        let (mut options, live) = self.start_options(&address);
        let (path, id) = self.bm_choice.clone();
        let mut selected = 0;
        if !path.is_empty() || id != 0 {
            let by_path = options.iter().position(|o| !path.is_empty() && o.1 == path);
            let found = by_path.or_else(|| options.iter().position(|o| id != 0 && o.2 == id));
            selected = match found {
                Some(at) => at,
                None => {
                    let shown = if path.is_empty() { format!("Channel {id}") } else { path.replace("\\/", "/") };
                    let label = if live { format!("{shown} (not on the server now)") } else { shown };
                    options.insert(1, (label, path, id));
                    1
                }
            };
        }
        let labels: Vec<SharedString> = options.iter().map(|o| o.0.as_str().into()).collect();
        sync_rows(&self.start_rows, labels);
        self.bm_options = options;
        w.settings.set_bm_channel(selected as i32);
        w.settings.set_bm_channel_hint(
            if live || address.is_empty() { "" } else { "Connect to this server to pick from its channels." }.into(),
        );
    }

    pub fn bookmark_channel_selected(&mut self, w: &Windows) {
        let index = w.settings.get_bm_channel().max(0) as usize;
        if let Some(option) = self.bm_options.get(index) {
            self.bm_choice = (option.1.clone(), option.2);
        }
    }

    fn start_here(&self) -> Option<(usize, String, u64, String, bool)> {
        let session = self.viewed_session().filter(|s| s.is_connected())?;
        let view = session.view.as_ref()?;
        let index = self.bookmarks.find_address(&session.request.address)?;
        let path = session::channel_path(view, view.own_channel)?;
        let bookmark = &self.bookmarks.items[index];
        let chosen = session::find_start_channel(view, &bookmark.channel, bookmark.channel_id);
        Some((index, path, view.own_channel, session.own_channel_name(), chosen == Some(view.own_channel)))
    }

    pub fn toggle_start_here(&mut self, w: &Windows) {
        let Some((index, path, channel, _, already)) = self.start_here() else {
            return;
        };
        let bookmark = &mut self.bookmarks.items[index];
        (bookmark.channel, bookmark.channel_id) = if already { (String::new(), 0) } else { (path, channel) };
        bookmark.channel_password.clear();
        self.save_bookmarks(w);
        self.dirty.sessions = true;
        if w.settings.get_bm_index() == index as i32 {
            self.bookmark_picked(w, index as i32);
        }
    }

    pub fn bookmark_picked(&mut self, w: &Windows, index: i32) {
        let Some(bookmark) = self.bookmarks.items.get(index.max(0) as usize) else {
            return;
        };
        w.settings.set_bm_index(index);
        w.settings.set_bm_name(bookmark.name.as_str().into());
        w.settings.set_bm_address(bookmark.address.as_str().into());
        w.settings.set_bm_nickname(bookmark.nickname.as_str().into());
        w.settings.set_bm_identity(
            self.identity_index(&bookmark.identity_uid).map(|i| i as i32).unwrap_or(self.default_identity()),
        );
        w.settings.set_bm_note("".into());
        w.settings.set_bm_server_password(bookmark.server_password.as_str().into());
        w.settings.set_bm_channel_password(bookmark.channel_password.as_str().into());
        w.settings.set_bm_auto(bookmark.auto_connect);
        self.bm_choice = (bookmark.channel.clone(), bookmark.channel_id);
        self.publish_start_channels(w);
    }

    pub fn bookmark_new(&mut self, w: &Windows) {
        w.settings.set_bm_index(-1);
        w.settings.set_bm_name("".into());
        w.settings.set_bm_address("".into());
        w.settings.set_bm_nickname(self.default_nickname().into());
        w.settings.set_bm_identity(self.default_identity());
        w.settings.set_bm_note("".into());
        w.settings.set_bm_server_password("".into());
        w.settings.set_bm_channel_password("".into());
        w.settings.set_bm_auto(false);
        self.bm_choice = (String::new(), 0);
        self.publish_start_channels(w);
    }

    pub fn bookmark_saved(&mut self, w: &Windows) {
        let address = w.settings.get_bm_address().trim().to_string();
        if address.is_empty() {
            w.settings.set_bm_note("Enter the server address.".into());
            return;
        }
        let typed_name = w.settings.get_bm_name().trim().to_string();
        let identity_uid = self
            .identities
            .get(w.settings.get_bm_identity().max(0) as usize)
            .map(|l| l.identity.uid())
            .unwrap_or_default();
        let bookmark = Bookmark {
            name: if typed_name.is_empty() { address.clone() } else { typed_name },
            address,
            nickname: w.settings.get_bm_nickname().trim().to_string(),
            identity_uid,
            channel: self.bm_choice.0.clone(),
            channel_id: self.bm_choice.1,
            server_password: w.settings.get_bm_server_password().to_string(),
            channel_password: if self.bm_choice.0.is_empty() && self.bm_choice.1 == 0 {
                String::new()
            } else {
                w.settings.get_bm_channel_password().to_string()
            },
            auto_connect: w.settings.get_bm_auto(),
        };
        let index = w.settings.get_bm_index();
        let saved_at = if index >= 0 && (index as usize) < self.bookmarks.items.len() {
            let duplicate = self.bookmarks.find_address(&bookmark.address).filter(|other| *other != index as usize);
            if duplicate.is_some() {
                w.settings.set_bm_note("Another bookmark already uses that address.".into());
                return;
            }
            self.bookmarks.items[index as usize] = bookmark;
            index as usize
        } else {
            self.bookmarks.upsert(bookmark)
        };
        self.save_bookmarks(w);
        self.bookmark_picked(w, saved_at as i32);
    }

    pub fn bookmark_removed(&mut self, w: &Windows) {
        let index = w.settings.get_bm_index();
        if index < 0 {
            return;
        }
        self.bookmarks.remove(index as usize);
        self.save_bookmarks(w);
        self.bookmark_new(w);
    }

    fn show_dialog(&mut self, w: &Windows, request: &ConnectRequest, error: Option<(DialogField, String)>) {
        w.main.set_dlg_address(request.address.as_str().into());
        w.main.set_dlg_nickname(request.nickname.as_str().into());
        w.main.set_dlg_password(request.password.as_str().into());
        w.main.set_dlg_identity(
            self.identity_index(&request.identity_uid).map(|i| i as i32).unwrap_or(self.default_identity()),
        );
        w.main.set_dlg_save(request.save_bookmark);
        let from_link = self.pending_link.as_ref().filter(|link| same_address(&link.address, &request.address));
        w.main.set_dlg_note(from_link.map(links::summary).unwrap_or_default().into());
        match error {
            Some((field, text)) => {
                w.main.set_dlg_error_field(field.index());
                w.main.set_dlg_error(text.into());
            }
            None => w.main.set_dlg_error("".into()),
        }
        w.main.set_menu_open(false);
        w.main.set_dialog_open(true);
    }

    pub fn open_connect(&mut self, w: &Windows) {
        self.pending_link = None;
        let request = ConnectRequest {
            nickname: self.default_nickname(),
            identity_uid: self.settings.identity_uid.clone(),
            save_bookmark: true,
            ..ConnectRequest::default()
        };
        self.show_dialog(w, &request, None);
    }

    pub fn connect_new(&mut self, w: &Windows) {
        let address = w.main.get_dlg_address().trim().to_string();
        let mut request = ConnectRequest {
            address: address.clone(),
            nickname: w.main.get_dlg_nickname().trim().to_string(),
            password: w.main.get_dlg_password().to_string(),
            save_bookmark: w.main.get_dlg_save(),
            ..ConnectRequest::default()
        };
        if address.is_empty() {
            self.show_dialog(w, &request, Some((DialogField::Address, "Enter the server address.".to_string())));
            return;
        }
        let index = w.main.get_dlg_identity();
        let Some(loaded) = (index >= 0).then(|| self.identities.get(index as usize)).flatten() else {
            self.show_dialog(
                w,
                &request,
                Some((
                    DialogField::Identity,
                    "You need an identity to connect. Create one in settings, under Identities.".to_string(),
                )),
            );
            return;
        };
        request.identity_uid = loaded.identity.uid();
        if let Some(existing) = self.bookmarks.find_address(&address) {
            request.name = self.bookmarks.items[existing].name.clone();
            if request.password.is_empty() {
                request.password = self.bookmarks.items[existing].server_password.clone();
            }
        }
        if let Some(link) = self.pending_link.take().filter(|link| same_address(&link.address, &request.address)) {
            request.channel = link.channel;
            request.channel_id = link.channel_id;
            request.channel_password = link.channel_password;
            request.token = link.token;
            if request.save_bookmark && request.name.is_empty() {
                request.name = link.bookmark;
            }
        }
        w.main.set_dialog_open(false);
        w.main.set_dlg_password("".into());
        w.main.set_dlg_error("".into());
        w.main.set_dlg_note("".into());
        self.settings.server_address = request.address.clone();
        if !request.nickname.is_empty() {
            self.settings.nickname = request.nickname.clone();
        }
        self.settings.identity_uid = request.identity_uid.clone();
        self.mark_settings_dirty();
        self.begin(w, request);
    }

    pub fn connect_bookmark(&mut self, w: &Windows, index: i32) {
        let Some(bookmark) = self.bookmarks.items.get(index.max(0) as usize).cloned() else {
            return;
        };
        let request = self.bookmark_request(&bookmark);
        self.begin(w, request);
    }

    fn bookmark_request(&self, bookmark: &Bookmark) -> ConnectRequest {
        let identity_uid = if self.identity_index(&bookmark.identity_uid).is_some() {
            bookmark.identity_uid.clone()
        } else {
            let index = self.default_identity();
            if index >= 0 { self.identities[index as usize].identity.uid() } else { String::new() }
        };
        ConnectRequest {
            name: bookmark.name.clone(),
            address: bookmark.address.clone(),
            nickname: if bookmark.nickname.trim().is_empty() { self.default_nickname() } else { bookmark.nickname.clone() },
            identity_uid,
            password: bookmark.server_password.clone(),
            channel: bookmark.channel.clone(),
            channel_id: bookmark.channel_id,
            channel_password: bookmark.channel_password.clone(),
            ..ConnectRequest::default()
        }
    }

    fn connect_from_start(&mut self, w: &Windows, start: &StartRequest) {
        let target = start.target.trim();
        if target.is_empty() {
            return;
        }
        let known = self
            .bookmarks
            .items
            .iter()
            .find(|b| b.name.trim().eq_ignore_ascii_case(target) || same_address(&b.address, target))
            .cloned();
        let mut request = match known {
            Some(bookmark) => self.bookmark_request(&bookmark),
            None => {
                let index = self.default_identity();
                ConnectRequest {
                    address: target.to_string(),
                    nickname: self.default_nickname(),
                    identity_uid: if index >= 0 { self.identities[index as usize].identity.uid() } else { String::new() },
                    ..ConnectRequest::default()
                }
            }
        };
        if !start.nickname.trim().is_empty() {
            request.nickname = start.nickname.trim().to_string();
        }
        if !start.channel.trim().is_empty() {
            request.channel = start.channel.trim().to_string();
            request.channel_id = 0;
        }
        self.begin(w, request);
    }

    fn begin(&mut self, w: &Windows, request: ConnectRequest) {
        let existing = self
            .sessions
            .iter()
            .find(|s| same_address(&s.request.address, &request.address) && s.request.identity_uid == request.identity_uid)
            .map(|s| s.id);
        if let Some(id) = existing {
            self.view_server(w, id);
            return;
        }
        let id = loop {
            let candidate = self.next_id;
            self.next_id = if self.next_id >= 0xfffe { 1 } else { self.next_id + 1 };
            if self.session(candidate).is_none() {
                break candidate;
            }
        };
        let mut session = Session::new(id, request.clone(), self.trace);
        session.folds.mode = FoldMode::from_index(self.settings.fold_mode);
        session.allow_whispers = self.settings.allow_whispers;
        session.push_line(ChatKind::System, "", &format!("Connecting to {}", request.address));
        self.sessions.push(session);
        match self.launch(id) {
            Ok(()) => self.view_server(w, id),
            Err(error) => {
                self.sessions.retain(|s| s.id != id);
                self.show_dialog(w, &request, Some(error));
            }
        }
        self.dirty = Dirty::everything();
    }

    fn launch(&mut self, id: u16) -> Result<(), (DialogField, String)> {
        let Some(position) = self.sessions.iter().position(|s| s.id == id) else {
            return Ok(());
        };
        let request = self.sessions[position].request.clone();
        let Some(index) = self.identity_index(&request.identity_uid) else {
            return Err((
                DialogField::Identity,
                "You need an identity to connect. Create one in settings, under Identities.".to_string(),
            ));
        };
        let mut options = ConnectOptions::new(&request.address, DEFAULT_PORT, self.identities[index].identity.clone());
        if !request.nickname.trim().is_empty() {
            options.nickname = request.nickname.trim().to_string();
        }
        options.server_password = request.password.clone();
        options.default_channel = request.channel.clone();
        options.default_channel_password = request.channel_password.clone();
        options.input_muted = self.mic_muted;
        options.output_muted = self.sound_muted;
        options.log_commands = self.trace;
        let (events_tx, events_rx) = mpsc::channel();
        let shared = self.engine.shared().clone();
        let allow_whispers = self.allow_whispers.clone();
        let sink: VoiceSink = Box::new(move |packet| {
            if packet.whisper && !allow_whispers.load(Ordering::Relaxed) {
                return;
            }
            if let Ok(mut playback) = shared.playback.lock() {
                playback.push(id, packet.client_id, packet.voice_id, packet.codec, packet.data);
            }
        });
        let client = ClientHandle::connect(options, events_tx, Some(sink));
        self.sessions[position].attach(client, events_rx);
        Ok(())
    }

    pub fn view_server(&mut self, w: &Windows, id: u16) {
        if self.session(id).is_none() {
            return;
        }
        if self.viewed != Some(id) {
            self.viewed = Some(id);
            self.chat_shown = None;
            self.own_talking = false;
            self.unheard = None;
            self.prompt = None;
            w.main.set_prompt_open(false);
            w.main.set_chat_input("".into());
        }
        self.route_mic();
        self.dirty = Dirty::everything();
    }

    pub fn disconnect_viewed(&mut self, w: &Windows) {
        let Some(id) = self.viewed else {
            return;
        };
        let attached = match self.session_mut(id) {
            Some(session) => {
                session.leaving = true;
                match &session.client {
                    Some(client) => {
                        client.disconnect("leaving");
                        true
                    }
                    None => false,
                }
            }
            None => return,
        };
        if !attached {
            self.finish_session(w, id, "disconnected", false);
        }
        self.dirty.sessions = true;
    }

    fn route_mic(&mut self) {
        let desired = self.viewed.filter(|id| self.session(*id).is_some_and(|s| s.is_connected()));
        let shared = self.engine.shared().clone();
        let paused = if desired == self.mic_target { None } else { shared.sink.lock().ok() };
        if let Some(mut slot) = paused {
            *slot = None;
            let on_air = shared.on_air_lane();
            if let MicMove::Switch { end_talk_on, send_to } = mic_move(self.mic_target, desired, on_air.is_some()) {
                if let Some(left) = end_talk_on.and_then(|id| self.session(id)) {
                    if let Some(client) = &left.client {
                        match on_air {
                            Some(lane) if lane != 0 => {
                                if let Ok(table) = self.lanes.lock() {
                                    if let Route::Whisper(target) = route(lane, &table) {
                                        client.send_whisper(target, CODEC_OPUS_VOICE, &[]);
                                    }
                                }
                            }
                            _ => client.send_voice(left.own_codec().0, &[]),
                        }
                    }
                }
                self.mic_target = send_to;
            }
            self.rebuild_lanes(true);
            let lanes = &self.lanes;
            *slot = self.mic_target.and_then(|id| self.session(id)).and_then(|s| s.client.clone()).map(|client| {
                let lanes = lanes.clone();
                let sink: FrameSink = Box::new(move |lane, codec, data| {
                    if lane == 0 {
                        client.send_voice(codec, data);
                    } else if let Ok(table) = lanes.lock() {
                        if let Route::Whisper(target) = route(lane, &table) {
                            client.send_whisper(target, codec, data);
                        }
                    }
                });
                sink
            });
        }
        if let Some(target) = self.mic_target.and_then(|id| self.session(id)) {
            let (codec, quality) = target.own_codec();
            self.engine.set_codec(codec, quality);
        }
    }

    fn apply_mute(&mut self, w: &Windows) {
        let (mic, sound) = (self.mic_muted, self.sound_muted);
        let target = self
            .mic_target
            .and_then(|id| self.session(id))
            .and_then(|s| s.client.clone().map(|client| (client, s.own_codec().0)));
        let clients: Vec<ClientHandle> =
            self.sessions.iter().filter(|s| s.is_connected()).filter_map(|s| s.client.clone()).collect();
        let engine = &self.engine;
        let lanes = &self.lanes;
        engine.pause_transmit(|| {
            let on_air = engine.shared().on_air_lane();
            engine.set_mic_muted(mic);
            engine.set_speaker_muted(sound);
            if let (true, Some(lane), Some((client, codec))) = (mic || sound, on_air, &target) {
                if lane == 0 {
                    client.send_voice(*codec, &[]);
                } else if let Ok(table) = lanes.lock() {
                    if let Route::Whisper(aim) = route(lane, &table) {
                        client.send_whisper(aim, CODEC_OPUS_VOICE, &[]);
                    }
                }
            }
            for client in &clients {
                client.set_mute_state(mic, sound);
            }
        });
        w.main.set_mic_muted(mic);
        w.main.set_sound_muted(sound);
    }

    pub fn toggle_mic(&mut self, w: &Windows) {
        self.mic_muted = !self.mic_muted;
        self.apply_mute(w);
        self.engine.play_cue(if self.mic_muted { Cue::MicOff } else { Cue::MicOn });
    }

    pub fn toggle_sound(&mut self, w: &Windows) {
        self.sound_muted = !self.sound_muted;
        self.apply_mute(w);
        self.engine.play_cue(if self.sound_muted { Cue::SoundOff } else { Cue::SoundOn });
    }

    fn remember_folds(&mut self, id: u16) {
        let Some(session) = self.session(id).filter(|s| !s.server_uid.is_empty()) else {
            return;
        };
        let uid = session.server_uid.clone();
        let chosen = session::remembered(&session.folds, session.view.as_ref());
        if chosen.is_empty() {
            self.settings.folds.remove(&uid);
        } else {
            let full = self.settings.folds.len() >= settings::MAX_REMEMBERED_SERVERS;
            if full && !self.settings.folds.contains_key(&uid) {
                let in_use = |known: &String| self.sessions.iter().any(|s| s.server_uid == *known);
                let spare = self.settings.folds.keys().find(|known| !in_use(known)).cloned();
                if let Some(spare) = spare {
                    self.settings.folds.remove(&spare);
                }
            }
            self.settings.folds.insert(uid, chosen);
        }
        self.mark_settings_dirty();
    }

    pub fn toggle_fold(&mut self, _w: &Windows, id: i32) {
        let Some(viewed) = self.viewed else {
            return;
        };
        let Some(session) = self.session_mut(viewed) else {
            return;
        };
        if let (Some(view), Ok(channel)) = (&session.view, u64::try_from(id)) {
            session::toggle_fold(view, &mut session.folds, channel);
            self.dirty.tree = true;
            self.remember_folds(viewed);
        }
    }

    pub fn ask_privilege_key(&mut self, w: &Windows) {
        let Some(session) = self.viewed_session().filter(|s| s.is_connected()) else {
            return;
        };
        self.key_prompt = Some(session.id);
        self.prompt = None;
        w.main.set_prompt_kind(1);
        w.main.set_prompt_name("".into());
        w.main.set_prompt_password("".into());
        w.main.set_prompt_open(true);
    }

    fn cannot_talk(&self) -> bool {
        let Some(view) = self.viewed_session().filter(|s| s.is_connected()).and_then(|s| s.view.as_ref()) else {
            return false;
        };
        let (Some(own), Some(node)) = (view.client(view.own_id), view.own_channel_node()) else {
            return false;
        };
        node.channel.needed_talk_power > 0 && own.talk_power < node.channel.needed_talk_power && !own.is_talker
    }

    pub fn person_ask(&mut self, w: &Windows) {
        let wanted = w.main.get_person_asking();
        if let Some(client) = self.viewed_session().filter(|s| s.is_connected()).and_then(|s| s.client.as_ref()) {
            client.request_talk(wanted, "");
        }
    }

    pub fn toggle_commander(&mut self, _w: &Windows) {
        let Some(session) = self.viewed_session().filter(|s| s.is_connected()) else {
            return;
        };
        let own = session.view.as_ref().and_then(|v| v.client(v.own_id));
        if let (Some(own), Some(client)) = (own, &session.client) {
            client.set_channel_commander(!own.is_channel_commander);
        }
    }

    pub fn view_changed(&mut self, w: &Windows) {
        let mode = w.settings.get_fold_mode().clamp(0, 2);
        if mode != self.settings.fold_mode {
            self.settings.fold_mode = mode;
            self.settings.folds.clear();
            for session in &mut self.sessions {
                session.folds.mode = FoldMode::from_index(mode);
                session.folds.chosen.clear();
            }
            self.mark_settings_dirty();
            self.dirty.tree = true;
        }
    }

    pub fn row_context(&mut self, w: &Windows, row: TreeRow) {
        match row.kind {
            3 => self.open_channel(w, row.id.max(0) as u64),
            4 => self.open_person(w, row.id.clamp(0, 0xffff) as u16),
            _ => {}
        }
    }

    fn open_channel(&mut self, w: &Windows, channel_id: u64) {
        let Some(session) = self.viewed_session().filter(|s| s.is_connected()) else {
            return;
        };
        let node = session.view.as_ref().and_then(|view| view.channels.iter().find(|n| n.channel.id == channel_id));
        let Some(node) = node else {
            return;
        };
        if let (false, Some(client)) = (node.channel.description_known, &session.client) {
            client.request_channel_description(channel_id);
        }
        self.channel_sheet = Some((session.id, channel_id));
        self.publish_channel(w);
        w.main.set_menu_open(false);
        w.main.set_channel_open(self.channel_sheet.is_some());
    }

    fn publish_channel(&mut self, w: &Windows) {
        let Some((session_id, channel_id)) = self.channel_sheet else {
            return;
        };
        let shown = self
            .session(session_id)
            .filter(|s| self.viewed == Some(s.id) && s.is_connected())
            .and_then(|s| s.view.as_ref())
            .and_then(|view| view.channels.iter().find(|n| n.channel.id == channel_id).map(|node| (view, node)));
        let Some((view, node)) = shown else {
            self.channel_sheet = None;
            w.main.set_channel_open(false);
            return;
        };
        let channel = &node.channel;
        let mut facts: Vec<String> = Vec::new();
        facts.push(match node.clients.len() {
            0 => "Nobody here".to_string(),
            1 => "1 person here".to_string(),
            n => format!("{n} people here"),
        });
        let kind = if channel.codec == ps_client::CODEC_OPUS_MUSIC { "Music" } else { "Voice" };
        facts.push(format!("{kind}, quality {}", channel.codec_quality));
        if channel.has_password {
            facts.push("locked".to_string());
        }
        if channel.needed_talk_power > 0 {
            facts.push("moderated".to_string());
        }
        if channel.max_clients > 0 {
            facts.push(format!("room for {}", channel.max_clients));
        }
        w.main.set_channel_title(channel.name.as_str().into());
        w.main.set_channel_topic(channel.topic.trim().into());
        w.main.set_channel_facts(facts.join("  \u{b7}  ").into());
        w.main.set_channel_text(session::plain_text(&channel.description).into());
        w.main.set_channel_current(channel.id == view.own_channel);
    }

    pub fn channel_join(&mut self, w: &Windows) {
        let Some((_, channel_id)) = self.channel_sheet.take() else {
            return;
        };
        w.main.set_channel_open(false);
        self.row_activated(w, TreeRow { kind: 3, id: channel_id as i32, ..TreeRow::default() });
    }

    pub fn row_activated(&mut self, w: &Windows, row: TreeRow) {
        if row.kind == 4 {
            self.open_person(w, row.id.clamp(0, 0xffff) as u16);
            return;
        }
        if row.kind != 3 {
            return;
        }
        let Some(session) = self.viewed_session() else {
            return;
        };
        let (Some(view), Some(client), true) = (&session.view, &session.client, session.is_connected()) else {
            return;
        };
        let Some(node) = view.channels.iter().find(|n| n.channel.id as i32 == row.id) else {
            return;
        };
        if node.channel.id == view.own_channel {
            return;
        }
        if node.channel.has_password {
            let prompt = (session.id, node.channel.id);
            w.main.set_prompt_kind(0);
            w.main.set_prompt_name(node.channel.name.as_str().into());
            w.main.set_prompt_password("".into());
            w.main.set_prompt_open(true);
            self.prompt = Some(prompt);
            self.key_prompt = None;
        } else {
            client.join_channel(node.channel.id, "");
        }
    }

    pub fn join_with_password(&mut self, w: &Windows) {
        let password = w.main.get_prompt_password().to_string();
        w.main.set_prompt_open(false);
        w.main.set_prompt_password("".into());
        w.main.set_prompt_kind(0);
        if let Some(session_id) = self.key_prompt.take() {
            let key = password.trim().to_string();
            let viewed = self.viewed == Some(session_id);
            if let (false, Some(session)) = (key.is_empty(), self.session_mut(session_id)) {
                if let Some(client) = session.client.clone() {
                    client.use_privilege_key(&key);
                    session.system("Privilege key sent. If the server accepts it, your groups change.");
                    self.dirty.chat |= viewed;
                }
            }
            return;
        }
        let Some((session_id, channel)) = self.prompt.take() else {
            return;
        };
        if let Some(client) = self.session(session_id).and_then(|s| s.client.as_ref()) {
            client.join_channel(channel, &password);
        }
    }

    fn open_person(&mut self, w: &Windows, client_id: u16) {
        let Some(session) = self.viewed_session().filter(|s| s.is_connected()) else {
            return;
        };
        let Some(view) = &session.view else {
            return;
        };
        let Some(person) = view.client(client_id) else {
            return;
        };
        let me = client_id == view.own_id;
        if let (false, Some(client)) = (me, &session.client) {
            client.request_details(client_id);
        }
        let voice = self.settings.voices.get(&person.uid).copied().unwrap_or(Voice::plain());
        w.main.set_person_volume(f32::from(voice.percent));
        w.main.set_person_muted(voice.muted);
        w.main.set_person_poke("".into());
        w.main.set_person_away(person.away);
        w.main.set_person_asking(person.talk_request);
        w.main.set_person_moderated(me && self.cannot_talk());
        w.main.set_person_away_message(person.away_message.as_str().into());
        self.person = Some((session.id, client_id));
        self.publish_person(w);
        w.main.set_menu_open(false);
        w.main.set_person_open(self.person.is_some());
    }

    fn publish_person(&mut self, w: &Windows) {
        let Some((session_id, client_id)) = self.person else {
            return;
        };
        let shown = self
            .session(session_id)
            .filter(|s| self.viewed == Some(s.id) && s.is_connected())
            .and_then(|s| s.view.as_ref().map(|view| (s, view)))
            .and_then(|(s, view)| view.client(client_id).map(|person| (s, view, person)));
        let Some((session, view, person)) = shown else {
            self.person = None;
            w.main.set_person_open(false);
            return;
        };
        let mut groups: Vec<&str> = session
            .server_groups
            .iter()
            .filter(|group| person.server_groups.contains(&group.id))
            .map(|group| group.name.as_str())
            .collect();
        if let Some(group) = session.channel_groups.iter().find(|group| group.id == person.channel_group) {
            if !groups.contains(&group.name.as_str()) {
                groups.push(group.name.as_str());
            }
        }
        let mut about: Vec<String> = Vec::new();
        let version = person.version.split_whitespace().next().unwrap_or("");
        match (version.is_empty(), person.platform.is_empty()) {
            (false, false) => about.push(format!("Client {version} on {}", person.platform)),
            (false, true) => about.push(format!("Client {version}")),
            (true, false) => about.push(person.platform.clone()),
            (true, true) => {}
        }
        if !person.country.trim().is_empty() {
            about.push(person.country.trim().to_uppercase());
        }
        if !person.description.trim().is_empty() {
            about.push(person.description.trim().to_string());
        }
        let mut status: Vec<String> = Vec::new();
        if person.away {
            let note = person.away_message.trim();
            status.push(if note.is_empty() { "Away".to_string() } else { format!("Away: {note}") });
        }
        if person.talk_request {
            let note = person.talk_request_message.trim();
            status.push(if note.is_empty() { "Asks to talk".to_string() } else { format!("Asks to talk: {note}") });
        }
        if person.is_recording {
            status.push("Recording".to_string());
        }
        if person.is_channel_commander {
            status.push("Channel commander".to_string());
        }
        if person.is_priority_speaker {
            status.push("Priority speaker".to_string());
        }
        w.main.set_person_name(person.nickname.as_str().into());
        w.main.set_person_me(client_id == view.own_id);
        w.main.set_person_line(groups.join(", ").into());
        w.main.set_person_about(about.join("  \u{b7}  ").into());
        w.main.set_person_status(status.join("  \u{b7}  ").into());
    }

    pub fn person_voice_changed(&mut self, w: &Windows) {
        let Some((session_id, client_id)) = self.person else {
            return;
        };
        let uid = self
            .session(session_id)
            .and_then(|s| s.view.as_ref())
            .and_then(|view| view.client(client_id))
            .map(|person| person.uid.clone())
            .filter(|uid| !uid.is_empty());
        let Some(uid) = uid else {
            return;
        };
        let voice = Voice {
            percent: w.main.get_person_volume().round().clamp(0.0, 200.0) as u16,
            muted: w.main.get_person_muted(),
        };
        if voice.is_plain() {
            self.settings.voices.remove(&uid);
        } else if self.settings.voices.len() < MAX_REMEMBERED_VOICES || self.settings.voices.contains_key(&uid) {
            self.settings.voices.insert(uid, voice);
        }
        self.mark_settings_dirty();
        let ids: Vec<u16> = self.sessions.iter().map(|s| s.id).collect();
        for id in ids {
            self.apply_voices(id);
        }
        self.dirty.tree = true;
    }

    fn apply_voices(&mut self, id: u16) {
        let Some(session) = self.session(id) else {
            return;
        };
        let Some(view) = &session.view else {
            return;
        };
        let mut wanted: HashMap<u16, (String, f32)> = HashMap::new();
        let mut silenced: HashSet<u16> = HashSet::new();
        for person in view.channels.iter().flat_map(|node| node.clients.iter()).filter(|c| c.id != view.own_id) {
            let voice = self.settings.voices.get(&person.uid).copied().unwrap_or(Voice::plain());
            if voice.muted {
                silenced.insert(person.id);
            }
            wanted.insert(person.id, (person.uid.clone(), voice.gain()));
        }
        let changed: Vec<(u16, f32)> = wanted
            .iter()
            .filter(|(client, entry)| session.voices_applied.get(*client) != Some(*entry))
            .map(|(client, entry)| (*client, entry.1))
            .collect();
        for (client, gain) in changed {
            self.engine.set_volume(id, client, gain);
        }
        if let Some(session) = self.session_mut(id) {
            session.voices_applied = wanted;
            session.silenced = silenced;
        }
    }

    pub fn person_message(&mut self, w: &Windows) {
        let Some((session_id, client_id)) = self.person.take() else {
            return;
        };
        w.main.set_person_open(false);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(person) = session.view.as_ref().and_then(|view| view.client(client_id)) else {
            return;
        };
        let peer = Peer { id: client_id, uid: person.uid.clone(), name: person.nickname.clone() };
        w.main.set_chat_peer(peer.name.as_str().into());
        session.peer = Some(peer);
        w.main.set_chat_target(2);
        w.main.set_chat_open(true);
        self.dirty.sessions = true;
    }

    pub fn person_poke(&mut self, w: &Windows) {
        let Some((session_id, client_id)) = self.person.take() else {
            return;
        };
        w.main.set_person_open(false);
        let text: String = w.main.get_person_poke().trim().chars().take(100).collect();
        w.main.set_person_poke("".into());
        let viewed = self.viewed == Some(session_id);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let name = session.view.as_ref().and_then(|view| view.client(client_id)).map(|person| person.nickname.clone());
        let (Some(name), Some(client)) = (name, session.client.clone()) else {
            return;
        };
        client.poke(client_id, &text);
        session.system(&if text.is_empty() { format!("You poked {name}") } else { format!("You poked {name}: {text}") });
        self.dirty.chat |= viewed;
    }

    pub fn person_away(&mut self, w: &Windows) {
        let away = w.main.get_person_away();
        let message: String = w.main.get_person_away_message().trim().chars().take(80).collect();
        for session in self.sessions.iter().filter(|s| s.is_connected()) {
            if let Some(client) = &session.client {
                client.set_away(away, if away { &message } else { "" });
            }
        }
    }

    pub fn send_chat(&mut self, w: &Windows) {
        let text = w.main.get_chat_input().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(session) = self.viewed_session() else {
            return;
        };
        let (Some(client), true) = (&session.client, session.is_connected()) else {
            return;
        };
        let target = if w.main.get_chat_open() { w.main.get_chat_target() } else { 0 };
        match (target, &session.peer) {
            (1, _) => client.send_text(TextTarget::Server, &text),
            (2, Some(peer)) => {
                let here = session.view.as_ref().and_then(|view| view.client(peer.id)).is_some_and(|c| c.uid == peer.uid);
                if !here {
                    w.main.set_notice(format!("{} is not connected any more.", peer.name).into());
                    return;
                }
                client.send_text(TextTarget::Client(peer.id), &text);
            }
            (2, None) => return,
            _ => client.send_text(TextTarget::Channel, &text),
        }
        w.main.set_chat_input("".into());
    }

    fn start_level_job(&mut self, uid: &str, level: u8) {
        if self.level_jobs.iter().any(|job| job.uid == uid && job.target >= level) {
            return;
        }
        let Some(index) = self.identity_index(uid) else {
            return;
        };
        let identity = self.identities[index].identity.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut identity = identity;
            let reached = identity.improve_security_level(level, &|| flag.load(Ordering::Relaxed));
            let _ = tx.send((identity.key_offset, reached));
        });
        self.level_jobs.push(LevelJob { uid: uid.to_string(), target: level, stop, result: rx });
    }

    fn poll_level_jobs(&mut self, w: &Windows) {
        let mut finished = Vec::new();
        self.level_jobs.retain(|job| match job.result.try_recv() {
            Ok((offset, reached)) => {
                finished.push((job.uid.clone(), offset, reached));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
        });
        for (uid, offset, reached) in finished {
            if !reached {
                continue;
            }
            if let Some(index) = self.identity_index(&uid) {
                if offset > self.identities[index].identity.key_offset {
                    self.identities[index].identity.key_offset = offset;
                }
                self.settings.key_offsets.insert(uid, offset);
                self.mark_settings_dirty();
                self.dirty.identities = true;
            }
        }
        for job in &self.level_jobs {
            let wanted =
                self.sessions.iter().any(|s| s.waiting_level.is_some() && s.request.identity_uid == job.uid);
            if !wanted {
                job.stop.store(true, Ordering::Relaxed);
            }
        }
        let waiting: Vec<(u16, u8, String)> = self
            .sessions
            .iter()
            .filter(|s| s.client.is_none() && !s.leaving)
            .filter_map(|s| s.waiting_level.map(|level| (s.id, level, s.request.identity_uid.clone())))
            .collect();
        for (id, level, uid) in waiting {
            if self.level_jobs.iter().any(|job| job.uid == uid) {
                continue;
            }
            let reached = self
                .identity_index(&uid)
                .is_some_and(|index| self.identities[index].identity.security_level() >= level);
            let relaunched = reached && {
                if let Some(session) = self.session_mut(id) {
                    session.waiting_level = None;
                    session.push_line(ChatKind::System, "", "Your identity is strong enough now. Connecting again.");
                }
                self.launch(id).is_ok()
            };
            if relaunched {
                self.dirty = Dirty::everything();
            } else {
                self.finish_session(w, id, &format!("the server requires identity security level {level}"), true);
            }
        }
    }

    fn finish_session(&mut self, w: &Windows, id: u16, reason: &str, was_connecting: bool) {
        let Some(position) = self.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        let order: Vec<(u16, bool)> = self.sessions.iter().map(|s| (s.id, s.is_connected())).collect();
        let session = self.sessions.remove(position);
        self.engine.clear_session(id);
        self.icons.forget_pending(&session.server_uid);
        if self.prompt.is_some_and(|(owner, _)| owner == id) {
            self.prompt = None;
            w.main.set_prompt_open(false);
        }
        let next = next_view(&order, self.viewed, id);
        if next != self.viewed {
            self.viewed = next;
            self.chat_shown = None;
            self.own_talking = false;
            w.main.set_chat_input("".into());
        }
        self.route_mic();
        self.dirty = Dirty::everything();
        if session.leaving {
            return;
        }
        if was_connecting {
            let (field, text) =
                connect_failure(reason, &session.request.address, !session.request.password.is_empty());
            if w.main.get_dialog_open() || session.request.quiet {
                w.main.set_notice(format!("{}: {text}", session.name).into());
            } else {
                self.show_dialog(w, &session.request, Some((field, text)));
            }
        } else {
            let reason = reason.trim().trim_end_matches('.');
            w.main.set_notice(format!("Disconnected from {}: {reason}.", session.name).into());
        }
    }

    fn on_outcome(&mut self, w: &Windows, id: u16, was_connecting: bool, outcome: Outcome) {
        let viewed = self.viewed == Some(id);
        if let Some(cue) = outcome.cue {
            self.engine.play_cue(cue);
        }
        for client in &outcome.forget {
            self.engine.remove_talker(id, *client);
        }
        if outcome.forget_all {
            self.engine.clear_session(id);
            if let Some(uid) = self.session(id).map(|s| s.server_uid.clone()) {
                self.icons.forget_pending(&uid);
            }
        }
        if let Some((icon, data)) = outcome.icon {
            self.icon_answered(id, icon, data);
        }
        self.dirty.tree |= viewed && (outcome.tree || outcome.talking.is_some());
        self.dirty.speakers |= outcome.tree || outcome.talking.is_some();
        self.dirty.chat |= viewed && outcome.chat;
        self.dirty.sessions |= outcome.header || outcome.connected;
        if outcome.channel && self.mic_target == Some(id) {
            self.route_mic();
        }
        if self.mic_target == Some(id) {
            if outcome.tree || outcome.whisper_from.is_some() {
                self.rebuild_lanes(false);
            }
            if outcome.whisper_unheard {
                self.note_unheard();
            }
        }
        if viewed && (outcome.tree || outcome.groups) && self.editor.is_some() {
            self.dirty.shortcuts = true;
        }
        if outcome.folds {
            self.remember_folds(id);
        }
        if outcome.tree {
            self.apply_voices(id);
        }
        if let Some((channel, locked)) = outcome.start_channel {
            self.enter_start_channel(w, id, channel, locked, &outcome.start_password);
        }
        if outcome.tree && w.settings.window().is_visible() && w.settings.get_tab() == 3 {
            self.dirty.bookmarks = true;
        }
        if let Some(text) = outcome.notice {
            w.main.set_notice(text.into());
        }
        if outcome.connected {
            self.on_connected(w, id);
        }
        if let Some(level) = outcome.level {
            if let Some(uid) = self.session(id).map(|s| s.request.identity_uid.clone()) {
                self.start_level_job(&uid, level);
            }
        }
        if outcome.retry {
            self.route_mic();
            self.dirty = Dirty::everything();
        }
        if let Some(reason) = outcome.closed {
            self.finish_session(w, id, &reason, was_connecting);
        }
    }

    fn icon_answered(&mut self, id: u16, icon: u32, data: Result<Vec<u8>, String>) {
        let Some(uid) = self.session(id).map(|s| s.server_uid.clone()) else {
            return;
        };
        match data {
            Ok(bytes) => {
                if let (Err(refused), true) = (self.icons.arrived(&uid, icon, &bytes), self.trace) {
                    if let Some(session) = self.session_mut(id) {
                        session.system(&format!("Icon {icon} is not shown: {}", refused.reason()));
                    }
                    self.dirty.chat |= self.viewed == Some(id);
                }
            }
            Err(_) => self.icons.failed(&uid, icon, Instant::now()),
        }
        self.dirty.tree |= self.viewed_session().is_some_and(|s| s.server_uid == uid);
        self.dirty.sessions |= self
            .sessions
            .iter()
            .any(|s| s.server_uid == uid && s.view.as_ref().is_some_and(|view| view.server.icon == icon));
    }

    fn picture(&mut self, uid: &str, icon: u32, client: Option<&ClientHandle>, now: Instant) -> Option<(slint::Image, bool)> {
        let found =
            if client.is_some() { self.icons.lookup(uid, icon, now) } else { self.icons.peek(uid, icon, now) };
        match found {
            Lookup::Standard(index) => self.standard_icons.get(index).map(|image| (image.clone(), true)),
            Lookup::Ready(image) => Some((image, false)),
            Lookup::Refresh(image) => {
                if let Some(client) = client {
                    client.request_icon(icon);
                }
                Some((image, false))
            }
            Lookup::Ask => {
                if let Some(client) = client {
                    client.request_icon(icon);
                }
                Some((slint::Image::default(), false))
            }
            Lookup::Waiting => Some((slint::Image::default(), false)),
            Lookup::Nothing => None,
        }
    }

    fn server_picture(&mut self, id: u16, now: Instant) -> Option<slint::Image> {
        let session = self.session(id)?;
        let icon = session.view.as_ref().map(|view| view.server.icon).filter(|icon| *icon != 0)?;
        let uid = session.server_uid.clone();
        let client = session.client.clone().filter(|_| session.is_connected());
        let (image, _) = self.picture(&uid, icon, client.as_ref(), now)?;
        Some(image).filter(|image| image.size().width > 0)
    }

    fn on_connected(&mut self, w: &Windows, id: u16) {
        let kept = self.session(id).and_then(|s| self.settings.folds.get(&s.server_uid)).cloned();
        let Some(session) = self.session_mut(id) else {
            return;
        };
        if let Some(kept) = kept {
            session.folds.chosen = kept.into_iter().collect();
        }
        let save = std::mem::take(&mut session.request.save_bookmark);
        let token = std::mem::take(&mut session.request.token);
        let request = session.request.clone();
        let name = session.name.clone();
        if let (true, Some(client)) = (self.mic_muted || self.sound_muted, self.session(id).and_then(|s| s.client.clone()))
        {
            client.set_mute_state(self.mic_muted, self.sound_muted);
        }
        if save {
            let kept = self.bookmarks.find_address(&request.address).map(|i| self.bookmarks.items[i].clone());
            let mut bookmark = kept.unwrap_or(Bookmark { name, ..Bookmark::default() });
            bookmark.address = request.address.clone();
            bookmark.nickname = request.nickname.clone();
            bookmark.identity_uid = request.identity_uid.clone();
            if bookmark.channel.is_empty() && bookmark.channel_id == 0 {
                bookmark.channel = request.channel.clone();
                bookmark.channel_id = request.channel_id;
            }
            self.bookmarks.upsert(bookmark);
            self.save_bookmarks(w);
        }
        if !token.is_empty() {
            if let Some(client) = self.session(id).and_then(|s| s.client.clone()) {
                client.use_privilege_key(&token);
            }
        }
        self.route_mic();
        self.dirty = Dirty::everything();
    }

    fn talk_key_wording(&self) -> String {
        let names: Vec<String> =
            self.settings.talk_keys.iter().map(|chord| chord_name(chord, &platform::key_char)).collect();
        match names.as_slice() {
            [] => "Choose a talk key in settings".to_string(),
            [one] => format!("Sends while I hold {one}"),
            [one, two] => format!("Sends while I hold {one} or {two}"),
            more => format!("Sends while I hold one of {} keys", more.len()),
        }
    }

    fn status_allows_whisper(&self) -> bool {
        self.viewed_session().is_some_and(|s| s.is_connected()) && !self.sound_muted && !self.mic_muted
    }

    fn speaker_rooms(&self) -> Vec<Room> {
        let mut rooms = Vec::new();
        for session in self.sessions.iter().filter(|s| s.is_connected()) {
            let Some(view) = &session.view else {
                continue;
            };
            let on_air = self.own_talking && self.mic_target == Some(session.id);
            let mut members = Vec::new();
            for node in &view.channels {
                let here = node.channel.id == view.own_channel;
                for person in node.clients.iter().filter(|person| !person.is_query) {
                    let me = person.id == view.own_id;
                    members.push(Member {
                        id: person.id,
                        name: person.nickname.clone(),
                        talking: if me { on_air } else { person.talking },
                        whispering: !me && person.whispering,
                        me,
                        here,
                    });
                }
            }
            rooms.push(Room { session: session.id, name: session.name.clone(), members });
        }
        rooms
    }

    fn publish_speakers(&mut self, w: &Windows) {
        let window = w.speakers.window();
        if !window.is_visible() {
            return;
        }
        let rooms = self.speaker_rooms();
        let linger = speakers::linger(self.settings.speakers_linger);
        let lines = self.roster.lines(&rooms, self.settings.speakers_all, linger, Instant::now());
        let height = window.size().to_logical(window.scale_factor()).height;
        let lines = speakers::fit(lines, speakers::capacity(height));
        self.speakers_empty = lines.is_empty();
        let rows = lines
            .into_iter()
            .map(|line| SpeakerRow {
                caption: line.caption,
                text: line.text.as_str().into(),
                talking: line.talking,
                whispering: line.whispering,
                me: line.me,
                extra: line.extra as i32,
            })
            .collect();
        sync_rows(&self.speaker_rows, rows);
    }

    fn speakers_start(&self, w: &Windows) -> Option<(i32, i32)> {
        let kept = self.settings.speakers_place.filter(|(x, y)| platform::on_a_screen(x + 24, y + 24));
        if kept.is_some() {
            return kept;
        }
        let main = w.main.window();
        if !main.is_visible() {
            return None;
        }
        let at = main.position();
        let size = main.size();
        let wide = (self.settings.speakers_size.0 * main.scale_factor()).round() as i32;
        let beside = (at.x + size.width as i32 + 12, at.y + 48);
        let over = (at.x + 48, at.y + 96);
        [beside, over]
            .into_iter()
            .find(|(x, y)| platform::on_a_screen(x + 24, y + 24) && platform::on_a_screen(x + wide - 24, y + 24))
    }

    fn apply_speakers(&mut self, w: &Windows) {
        w.speakers.set_locked(self.settings.speakers_locked);
        w.speakers.set_on_top(self.settings.speakers_on_top);
        w.main.set_speakers_shown(self.settings.speakers_shown);
        w.main.set_speakers_locked(self.settings.speakers_locked);
        w.settings.set_sp_shown(self.settings.speakers_shown);
        w.settings.set_sp_locked(self.settings.speakers_locked);
        w.settings.set_sp_on_top(self.settings.speakers_on_top);
        w.settings.set_sp_all(self.settings.speakers_all);
        w.settings.set_sp_opacity(self.settings.speakers_opacity);
        w.settings.set_sp_linger(self.settings.speakers_linger as f32);
        let window = w.speakers.window();
        if self.settings.speakers_shown && !window.is_visible() {
            let (width, height) = self.settings.speakers_size;
            window.set_size(slint::LogicalSize::new(width, height));
            if let Some((x, y)) = self.speakers_start(w) {
                window.set_position(slint::PhysicalPosition::new(x, y));
            }
            let front = platform::own_front_window();
            let _ = w.speakers.show();
            platform::bring_front(front);
        } else if !self.settings.speakers_shown && window.is_visible() {
            self.remember_speakers(w);
            let _ = w.speakers.hide();
        }
        self.speakers_ticks = 0;
        self.speakers_look = None;
        self.dirty.speakers = true;
    }

    fn remember_speakers(&mut self, w: &Windows) -> bool {
        let window = w.speakers.window();
        if !window.is_visible() || window.is_minimized() {
            return false;
        }
        let at = window.position();
        let size = window.size().to_logical(window.scale_factor());
        if size.width < 1.0 || size.height < 1.0 {
            return false;
        }
        let place = Some((at.x, at.y));
        let kept = self.settings.speakers_size;
        let sized = (size.width - kept.0).abs() > 0.5 || (size.height - kept.1).abs() > 0.5;
        if place == self.settings.speakers_place && !sized {
            return false;
        }
        self.settings.speakers_place = place;
        self.settings.speakers_size = (size.width, size.height);
        self.mark_settings_dirty();
        sized
    }

    fn watch_speakers(&mut self, w: &Windows) {
        if !w.speakers.window().is_visible() {
            return;
        }
        if self.roster.due(Instant::now()) {
            self.dirty.speakers = true;
        }
        let locked = self.settings.speakers_locked;
        let solid = (self.settings.speakers_opacity.clamp(20.0, 100.0) * 2.55).round() as u8;
        let look = (if locked && self.speakers_empty { 0 } else { solid }, locked);
        if self.speakers_look != Some(look) || self.speakers_ticks % 8 == 0 {
            let done = platform::overlay_style(SPEAKERS_TITLE, look.0, look.1);
            self.speakers_look = done.then_some(look);
        }
        self.speakers_ticks = self.speakers_ticks.wrapping_add(1);
        if self.speakers_ticks >= 16 && self.remember_speakers(w) {
            self.dirty.speakers = true;
        }
    }

    pub fn speakers_dragged(&mut self, w: &Windows, dx: f32, dy: f32) {
        if self.settings.speakers_locked {
            return;
        }
        let window = w.speakers.window();
        let scale = window.scale_factor();
        let (right, down) = ((dx * scale).round() as i32, (dy * scale).round() as i32);
        if (right, down) != (0, 0) {
            let at = window.position();
            window.set_position(slint::PhysicalPosition::new(at.x + right, at.y + down));
        }
    }

    pub fn speakers_sized(&mut self, w: &Windows, width: f32, height: f32) {
        if self.settings.speakers_locked {
            return;
        }
        let window = w.speakers.window();
        let size = window.size().to_logical(window.scale_factor());
        let width = width.clamp(speakers::MIN_WIDTH, speakers::MAX_SIDE);
        let height = height.clamp(speakers::MIN_HEIGHT, speakers::MAX_SIDE);
        if (width - size.width).abs() >= 0.5 || (height - size.height).abs() >= 0.5 {
            window.set_size(slint::LogicalSize::new(width, height));
        }
    }

    pub fn park_speakers(&mut self, w: &Windows) {
        self.remember_speakers(w);
        let _ = w.speakers.hide();
    }

    pub fn speakers_toggle(&mut self, w: &Windows) {
        self.settings.speakers_shown = !self.settings.speakers_shown;
        self.mark_settings_dirty();
        self.apply_speakers(w);
    }

    pub fn speakers_lock(&mut self, w: &Windows) {
        self.settings.speakers_locked = !self.settings.speakers_locked;
        self.mark_settings_dirty();
        self.apply_speakers(w);
    }

    pub fn speakers_closed(&mut self, w: &Windows) {
        self.settings.speakers_shown = false;
        self.mark_settings_dirty();
        self.apply_speakers(w);
    }

    pub fn speakers_changed(&mut self, w: &Windows) {
        self.settings.speakers_shown = w.settings.get_sp_shown();
        self.settings.speakers_locked = w.settings.get_sp_locked();
        self.settings.speakers_on_top = w.settings.get_sp_on_top();
        self.settings.speakers_all = w.settings.get_sp_all();
        self.settings.speakers_opacity = w.settings.get_sp_opacity().clamp(20.0, 100.0);
        let linger = w.settings.get_sp_linger().round().clamp(0.0, speakers::MAX_LINGER_SECONDS as f32);
        self.settings.speakers_linger = linger as u32;
        self.mark_settings_dirty();
        self.apply_speakers(w);
    }

    fn follow_scale(&mut self, w: &Windows) {
        let windows = [w.main.window(), w.settings.window(), w.speakers.window()];
        for (index, window) in windows.into_iter().enumerate() {
            let scale = window.scale_factor();
            let size = window.size().to_logical(scale);
            let free = !window.is_maximized() && !window.is_fullscreen() && !window.is_minimized();
            match self.scales[index].step(scale, size.width, size.height, free) {
                Step::Nothing => {}
                Step::Refresh => {
                    let nudge = self.scales[index].nudge();
                    match index {
                        0 => w.main.set_scale_nudge(nudge),
                        1 => w.settings.set_scale_nudge(nudge),
                        _ => w.speakers.set_scale_nudge(nudge),
                    }
                }
                Step::Restore(width, height) => window.set_size(slint::LogicalSize::new(width, height)),
            }
        }
    }

    fn retry_lost_connections(&mut self, w: &Windows) {
        let now = Instant::now();
        let mut due: Vec<u16> = Vec::new();
        for session in &mut self.sessions {
            let Some(at) = session.retry_at.filter(|_| session.client.is_none()) else {
                continue;
            };
            if now >= at {
                session.retry_at = None;
                session.state_text = "Connecting again".to_string();
                due.push(session.id);
                self.dirty.sessions = true;
                continue;
            }
            let text = format!("Connection lost. Trying again in {} s", at.duration_since(now).as_secs() + 1);
            if session.state_text != text {
                session.state_text = text;
                self.dirty.sessions = true;
            }
        }
        for id in due {
            if let Err((_, problem)) = self.launch(id) {
                self.finish_session(w, id, &problem, false);
            }
        }
    }

    fn status_text(&self) -> String {
        let Some(session) = self.viewed_session() else {
            return "Not connected".to_string();
        };
        if !session.is_connected() {
            let told = session.waiting_level.is_some() || session.retries > 0;
            return if told { session.state_text.clone() } else { "Connecting".to_string() };
        }
        if self.sound_muted {
            return "Sound muted".to_string();
        }
        if self.mic_muted {
            return "Microphone muted".to_string();
        }
        if self.cannot_talk() {
            return "No permission to talk here yet".to_string();
        }
        if self.own_talking {
            return "Talking".to_string();
        }
        match self.settings.tx_mode {
            1 => self.talk_key_wording(),
            2 => "Always sends".to_string(),
            _ => "Sends when I speak".to_string(),
        }
    }

    pub fn tick(&mut self, w: &Windows) {
        let ids: Vec<u16> = self.sessions.iter().map(|s| s.id).collect();
        for id in ids {
            let events = self.session_mut(id).map(|s| s.drain()).unwrap_or_default();
            for event in events {
                let Some(session) = self.session_mut(id) else {
                    break;
                };
                let was_connecting = !session.is_connected();
                let outcome = session.apply(event);
                self.on_outcome(w, id, was_connecting, outcome);
            }
        }
        self.take_wishes(w);
        self.follow_scale(w);
        self.watch_speakers(w);
        self.retry_lost_connections(w);
        self.poll_level_jobs(w);
        self.poll_capture(w);
        let fired = self.watcher.state().take_fired();
        if fired & 1 != 0 {
            self.toggle_mic(w);
        }
        if fired & 2 != 0 {
            self.toggle_sound(w);
        }
        self.watch_whispers();

        let level = self.engine.shared().input_level();
        let position = level_position(level);
        let transmitting = self.engine.is_transmitting();
        let on_air = transmitting && self.mic_target.is_some() && !self.whisper_goes_nowhere();
        if on_air != self.own_talking {
            self.own_talking = on_air;
            self.dirty.tree = true;
        }
        w.main.set_input_level(position);
        w.main.set_transmitting(on_air);
        w.settings.set_input_level(position);
        w.settings.set_transmitting(
            transmitting || (self.settings.tx_mode == 0 && level >= self.settings.vad_threshold),
        );

        let whisper = self.whisper_status().filter(|_| self.status_allows_whisper());
        let wide = whisper.is_some();
        let status = whisper.unwrap_or_else(|| self.status_text());
        if status != self.shown_status || wide != self.shown_wide {
            w.main.set_status_text(status.as_str().into());
            w.main.set_status_wide(wide);
            self.shown_status = status;
            self.shown_wide = wide;
        }

        let echo = if !self.settings.echo_cancel {
            "Turn this on when you listen through speakers, so that others do not hear themselves. A headset does not need it. It delays your voice by about 11 ms."
                .to_string()
        } else {
            match self.engine.shared().echo_reduction() {
                Some(db) if db >= 3.0 => {
                    format!("On. Taking about {:.0} dB of the speakers' sound out of your microphone.", db.min(60.0))
                }
                Some(_) => "On. Listening to what the speakers play.".to_string(),
                None => "On. Nothing is playing right now.".to_string(),
            }
        };
        if echo != self.shown_echo {
            w.settings.set_echo_status(echo.as_str().into());
            self.shown_echo = echo;
        }

        let devices = self.engine.status();
        let now = Instant::now();
        let silent = devices.input_ok && !self.mic_muted && level <= SILENCE_DB;
        if silent {
            self.silent_since.get_or_insert(now);
        } else {
            self.silent_since = None;
        }
        let silent_long = self.silent_since.is_some_and(|since| now.duration_since(since) >= SILENCE_HINT_AFTER);
        let mic_text = if devices.input.is_empty() {
            "Opening the microphone…".to_string()
        } else if silent_long {
            format!(
                "{} is sending silence. Check that it is not muted on the headset or in Windows.",
                devices.input
            )
        } else if devices.input_ok {
            format!("Using {}", devices.input)
        } else {
            devices.input.clone()
        };
        let sound_text = if devices.output.is_empty() {
            "Opening the speakers…".to_string()
        } else if devices.output_ok {
            format!("Using {}", devices.output)
        } else {
            devices.output.clone()
        };
        if (mic_text.as_str(), sound_text.as_str()) != (self.shown_devices.0.as_str(), self.shown_devices.1.as_str()) {
            w.settings.set_mic_status(mic_text.as_str().into());
            w.settings.set_sound_status(sound_text.as_str().into());
            self.shown_devices = (mic_text, sound_text);
        }
        if silent_long && !self.silence_warned && self.mic_target.is_some() {
            self.silence_warned = true;
            w.main.set_notice(
                "Your microphone is sending silence. Check that it is not muted on the headset or in Windows.".into(),
            );
        }

        if let Some(codec) = self.engine.take_unsupported_codec() {
            if !self.warned_codecs.contains(&codec) {
                self.warned_codecs.push(codec);
                let notice = if codec == CODEC_CELT_MONO {
                    "Someone is talking with the old CELT voice format, which PhishSpeak cannot play."
                } else {
                    "Someone is talking with a voice format PhishSpeak does not know."
                };
                w.main.set_notice(notice.into());
            }
        }

        if self.save_at.is_some_and(|at| now >= at) {
            self.save_at = None;
            let _ = self.settings.save();
        }
    }

    fn publish_identities(&mut self, w: &Windows) {
        let names: Vec<String> = self
            .identities
            .iter()
            .map(|l| {
                let label = if l.identity.name.trim().is_empty() { &l.identity.nickname } else { &l.identity.name };
                let uid = l.identity.uid();
                let short: String = uid.chars().take(8).collect();
                format!("{label} ({short}…)")
            })
            .collect();
        let model = string_model(names);
        w.main.set_identity_names(model.clone());
        w.settings.set_identity_names(model);
        let rows: Vec<IdentityRow> = self
            .identities
            .iter()
            .map(|l| IdentityRow {
                name: if l.identity.name.trim().is_empty() {
                    l.identity.nickname.as_str().into()
                } else {
                    l.identity.name.as_str().into()
                },
                uid: l.identity.uid().into(),
                detail: format!(
                    "Security level {} · {}",
                    l.identity.security_level(),
                    l.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                )
                .into(),
            })
            .collect();
        w.settings.set_identities(ModelRc::new(VecModel::from(rows)));
    }

    fn publish_sessions(&mut self, w: &Windows) {
        let now = Instant::now();
        let ids: Vec<u16> = self.sessions.iter().map(|s| s.id).collect();
        let pictures: Vec<Option<slint::Image>> = ids.iter().map(|id| self.server_picture(*id, now)).collect();
        let viewed_picture =
            self.viewed.and_then(|viewed| ids.iter().position(|id| *id == viewed)).and_then(|at| pictures[at].clone());
        w.main.set_has_server_icon(viewed_picture.is_some());
        w.main.set_server_icon(viewed_picture.unwrap_or_default());
        let tiles: Vec<ServerTile> = self
            .sessions
            .iter()
            .zip(pictures)
            .map(|(s, picture)| ServerTile {
                id: i32::from(s.id),
                initials: initials(&s.name).into(),
                name: s.name.as_str().into(),
                detail: if !s.is_connected() {
                    s.short_detail().into()
                } else if self.viewed == Some(s.id) {
                    "viewing".into()
                } else if s.talkers > 0 {
                    "someone is talking".into()
                } else {
                    s.short_detail().into()
                },
                viewed: self.viewed == Some(s.id),
                connected: s.is_connected(),
                talking: s.talkers > 0,
                has_icon: picture.is_some(),
                icon: picture.unwrap_or_default(),
            })
            .collect();
        let others: Vec<ServerTile> = tiles.iter().filter(|t| !t.viewed).take(HEADER_TILES).cloned().collect();
        w.main.set_sessions(ModelRc::new(VecModel::from(tiles)));
        w.main.set_others(ModelRc::new(VecModel::from(others)));
        let own = self
            .viewed_session()
            .filter(|s| s.is_connected())
            .and_then(|s| s.view.as_ref())
            .and_then(|v| v.client(v.own_id));
        w.main.set_viewed_connected(own.is_some());
        w.main.set_start_here(match self.start_here() {
            Some((_, _, _, name, true)) => format!("Stop starting in {name}").into(),
            Some((_, _, _, name, false)) => format!("Start in {name} next time").into(),
            None => SharedString::new(),
        });
        w.main.set_commander(own.is_some_and(|c| c.is_channel_commander));
        let nickname = match self.viewed_session() {
            Some(session) => {
                w.main.set_viewing(true);
                w.main.set_server_name(session.name.as_str().into());
                w.main.set_server_detail(session.detail().into());
                w.main.set_server_initials(initials(&session.name).into());
                w.main.set_channel_name(session.own_channel_name().into());
                w.main.set_ping_text(session.ping.as_str().into());
                let own = session.own_nickname();
                if own.trim().is_empty() { self.default_nickname() } else { own }
            }
            None => {
                w.main.set_viewing(false);
                w.main.set_server_name("".into());
                w.main.set_server_detail("".into());
                w.main.set_server_initials("".into());
                w.main.set_channel_name("".into());
                w.main.set_ping_text("".into());
                self.default_nickname()
            }
        };
        let peer = self.viewed_session().and_then(|s| s.peer.as_ref()).map(|peer| peer.name.clone()).unwrap_or_default();
        if peer.is_empty() && w.main.get_chat_target() == 2 {
            w.main.set_chat_target(0);
        }
        w.main.set_chat_peer(peer.into());
        w.main.set_initials(initials(&nickname).into());
        w.main.set_nickname(nickname.into());
        w.main.set_menu_bookmarks(ModelRc::new(VecModel::from(self.bookmark_rows(true))));
    }

    fn publish_bookmarks(&mut self, w: &Windows) {
        let rows = self.bookmark_rows(false);
        w.main.set_bookmarks(ModelRc::new(VecModel::from(rows.clone())));
        w.settings.set_bookmarks(ModelRc::new(VecModel::from(rows)));
        self.publish_start_channels(w);
        w.main.set_menu_bookmarks(ModelRc::new(VecModel::from(self.bookmark_rows(true))));
    }

    fn publish_tree(&mut self) {
        let now = Instant::now();
        let (data, uid, client) = match self.viewed_session() {
            Some(s) => (
                s.view.as_ref().map(|view| build_rows_folded(view, self.own_talking, &s.folds)).unwrap_or_default(),
                s.server_uid.clone(),
                s.client.clone().filter(|_| s.is_connected()),
            ),
            None => (Vec::new(), String::new(), None),
        };
        let mut rows: Vec<TreeRow> = Vec::with_capacity(data.len());
        let silenced = self.viewed_session().map(|s| s.silenced.clone()).unwrap_or_default();
        for row in &data {
            let mut shown = tree_row(row);
            if row.kind == RowKind::Person && row.tag.is_empty() && silenced.contains(&(row.id as u16)) {
                shown.tag = "muted by you".into();
            }
            for icon in &row.icons {
                if let Some((picture, tinted)) = self.picture(&uid, *icon, client.as_ref(), now) {
                    set_badge(&mut shown, picture, tinted);
                }
            }
            rows.push(shown);
        }
        if self.tree_shown == self.viewed {
            sync_rows(&self.tree, rows);
        } else {
            self.tree.set_vec(rows);
            self.tree_shown = self.viewed;
        }
    }

    fn publish_chat(&mut self, w: &Windows) {
        let Some(session) = self.viewed_session() else {
            self.chat.set_vec(Vec::<ChatRow>::new());
            self.chat_shown = None;
            w.main.set_has_chat(false);
            return;
        };
        let (id, total, length) = (session.id, session.chat_total, session.chat.len());
        let fresh = match self.chat_shown {
            Some((shown_id, shown_total)) if shown_id == id && total >= shown_total => {
                Some((total - shown_total) as usize).filter(|added| *added <= length)
            }
            _ => None,
        };
        match fresh {
            Some(added) => {
                for line in session.chat.iter().skip(length - added) {
                    self.chat.push(chat_row(line));
                }
                while self.chat.row_count() > length {
                    self.chat.remove(0);
                }
            }
            None => self.chat.set_vec(session.chat.iter().map(chat_row).collect::<Vec<ChatRow>>()),
        }
        match session.chat.back() {
            Some(line) => {
                w.main.set_last_chat(chat_row(line));
                w.main.set_has_chat(true);
            }
            None => w.main.set_has_chat(false),
        }
        self.chat_shown = Some((id, total));
    }

    pub fn refresh(&mut self, w: &Windows) {
        let dirty = std::mem::take(&mut self.dirty);
        if dirty.identities {
            self.publish_identities(w);
        }
        if dirty.bookmarks || dirty.identities {
            self.publish_bookmarks(w);
        }
        if dirty.sessions {
            self.publish_sessions(w);
        }
        if dirty.tree {
            self.publish_tree();
            self.publish_person(w);
            self.publish_channel(w);
        }
        if dirty.speakers || dirty.tree || dirty.sessions {
            self.publish_speakers(w);
        }
        if dirty.chat {
            self.publish_chat(w);
        }
        if dirty.shortcuts {
            self.publish_shortcuts(w);
        }
    }

    pub fn shutdown(&mut self) {
        self.engine.set_frame_sink(None);
        self.engine.set_loopback(false);
        for job in &self.level_jobs {
            job.stop.store(true, Ordering::Relaxed);
        }
        let mut clients = Vec::new();
        for session in &mut self.sessions {
            session.leaving = true;
            if let Some(client) = &session.client {
                client.disconnect("leaving");
                clients.push(client.clone());
            }
        }
        let deadline = Instant::now() + Duration::from_millis(900);
        while clients.iter().any(|c| !c.is_closed()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.settings.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synced(old: &[i32], new: &[i32]) -> Vec<i32> {
        let model = VecModel::from(old.to_vec());
        sync_rows(&model, new.to_vec());
        (0..model.row_count()).filter_map(|index| model.row_data(index)).collect()
    }

    #[test]
    fn rows_are_patched_where_they_changed() {
        let cases: [(&[i32], &[i32]); 12] = [
            (&[], &[]),
            (&[], &[1, 2, 3]),
            (&[1, 2, 3], &[]),
            (&[1, 2, 3], &[1, 2, 3]),
            (&[1, 2, 3], &[1, 9, 3]),
            (&[1, 2, 3], &[1, 3]),
            (&[1, 3], &[1, 2, 3]),
            (&[1, 2, 3, 4, 5], &[1, 5]),
            (&[1, 5], &[1, 2, 3, 4, 5]),
            (&[1, 1, 1], &[1, 1]),
            (&[1, 2, 3], &[7, 8, 9, 10]),
            (&[4, 5, 6, 7], &[6, 7, 4, 5, 6]),
        ];
        for (old, new) in cases {
            assert_eq!(synced(old, new), new, "{old:?} -> {new:?}");
        }
        assert_eq!(level_position(-70.0), 0.0);
        assert_eq!(level_position(0.0), 1.0);
    }
}
