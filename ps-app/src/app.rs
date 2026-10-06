use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ps_client::spacer::{SpacerAlign, SpacerLine};
use ps_client::{ClientHandle, ConnectOptions, TextTarget, VoiceSink, DEFAULT_PORT};
use ps_identity::Identity;
use ps_voice::{AudioEngine, DeviceInfo, FrameSink, TxMode};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

use crate::bookmarks::{initials, Bookmark, Bookmarks};
use crate::platform;
use crate::session::{
    build_rows, connect_failure, mic_move, next_view, ChannelIcon, ChatKind, ChatLine, ConnectRequest, DialogField,
    MicMove, Outcome, RowData, RowKind, Session,
};
use crate::settings::{self, Settings};
use crate::{BookmarkRow, ChatRow, IdentityRow, PhishSpeakApp, ServerTile, SettingsWindow, TreeRow};

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
    if model.row_count() == rows.len() {
        for (index, row) in rows.into_iter().enumerate() {
            if model.row_data(index).as_ref() != Some(&row) {
                model.set_row_data(index, row);
            }
        }
    } else {
        model.set_vec(rows);
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
    }
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

pub struct Windows {
    pub main: PhishSpeakApp,
    pub settings: SettingsWindow,
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
}

impl Dirty {
    fn everything() -> Self {
        Self { sessions: true, tree: true, chat: true, bookmarks: true, identities: true }
    }
}

pub struct App {
    main: Weak<PhishSpeakApp>,
    settings_window: Weak<SettingsWindow>,
    engine: AudioEngine,
    pub settings: Settings,
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
    dirty: Dirty,
    silent_since: Option<Instant>,
    silence_warned: bool,
    warned_codecs: Vec<u8>,
    shown_status: String,
    shown_devices: (String, String),
    trace: bool,
}

pub fn with_app(app: &Rc<RefCell<App>>, f: impl FnOnce(&mut App, &Windows)) {
    let Ok(mut state) = app.try_borrow_mut() else {
        return;
    };
    let (Some(main), Some(settings)) = (state.main.upgrade(), state.settings_window.upgrade()) else {
        return;
    };
    let windows = Windows { main, settings };
    f(&mut state, &windows);
    state.refresh(&windows);
}

impl App {
    pub fn new(main: &PhishSpeakApp, settings_window: &SettingsWindow, settings: Settings) -> Self {
        let engine = AudioEngine::start();
        if !settings.input_device.is_empty() {
            engine.set_input_device(Some(settings.input_device.clone()));
        }
        if !settings.output_device.is_empty() {
            engine.set_output_device(Some(settings.output_device.clone()));
        }
        Self {
            main: main.as_weak(),
            settings_window: settings_window.as_weak(),
            engine,
            settings,
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
            dirty: Dirty::everything(),
            silent_since: None,
            silence_warned: false,
            warned_codecs: Vec::new(),
            shown_status: String::new(),
            shown_devices: (String::new(), String::new()),
            trace: std::env::var_os("PHISHSPEAK_TRACE").is_some(),
        }
    }

    pub fn start(&mut self, w: &Windows, requests: &[StartRequest]) {
        w.main.set_tree(ModelRc::from(self.tree.clone()));
        w.main.set_chat(ModelRc::from(self.chat.clone()));
        w.settings.set_ptt_keys(string_model(platform::PTT_KEYS.iter().map(|k| k.0.to_string()).collect()));
        w.settings.set_version(env!("CARGO_PKG_VERSION").into());
        w.settings.set_tx_mode(self.settings.tx_mode);
        w.settings.set_vad_threshold(self.settings.vad_threshold);
        w.settings.set_mic_gain(self.settings.mic_gain);
        w.settings.set_output_volume(self.settings.output_volume);
        w.settings.set_ptt_key_index(self.settings.ptt_key.clamp(0, platform::PTT_KEYS.len() as i32 - 1));
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
        self.save_at = None;
        self.dirty = Dirty::everything();
        for request in requests {
            self.connect_from_start(w, request);
        }
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
        let key = w.settings.get_ptt_key_index().clamp(0, platform::PTT_KEYS.len() as i32 - 1);
        let shared = self.engine.shared();
        shared.set_tx_mode(TxMode::from_index(tx_mode as u8));
        shared.set_vad_threshold(threshold);
        shared.set_input_gain(gain / 100.0);
        shared.set_output_volume(volume / 100.0);
        self.engine.set_loopback(w.settings.get_mic_test());
        w.main.set_threshold_position(if tx_mode == 0 { level_position(threshold) } else { -1.0 });
        self.settings.tx_mode = tx_mode;
        self.settings.vad_threshold = threshold;
        self.settings.mic_gain = gain;
        self.settings.output_volume = volume;
        self.settings.ptt_key = key;
        self.mark_settings_dirty();
    }

    pub fn open_settings(&mut self, w: &Windows, tab: i32) {
        w.settings.set_tab(tab.clamp(0, 5));
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

    pub fn close_settings(&mut self, w: &Windows) {
        self.stop_mic_test(w);
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
    }

    pub fn bookmark_new(&mut self, w: &Windows) {
        w.settings.set_bm_index(-1);
        w.settings.set_bm_name("".into());
        w.settings.set_bm_address("".into());
        w.settings.set_bm_nickname(self.default_nickname().into());
        w.settings.set_bm_identity(self.default_identity());
        w.settings.set_bm_note("".into());
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
        }
        w.main.set_dialog_open(false);
        w.main.set_dlg_password("".into());
        w.main.set_dlg_error("".into());
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
        request.channel = start.channel.trim().to_string();
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
        options.input_muted = self.mic_muted;
        options.output_muted = self.sound_muted;
        options.log_commands = self.trace;
        let (events_tx, events_rx) = mpsc::channel();
        let shared = self.engine.shared().clone();
        let sink: VoiceSink = Box::new(move |packet| {
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
        if let MicMove::Switch { end_talk_on, send_to } =
            mic_move(self.mic_target, desired, self.engine.is_transmitting())
        {
            let sink = send_to.and_then(|id| self.session(id)).and_then(|s| s.client.clone()).map(|client| {
                let sink: FrameSink = Box::new(move |codec, data| client.send_voice(codec, data));
                sink
            });
            self.engine.set_frame_sink(sink);
            if let Some(left) = end_talk_on.and_then(|id| self.session(id)) {
                if let Some(client) = &left.client {
                    client.send_voice(left.own_codec().0, &[]);
                }
            }
            self.mic_target = send_to;
        }
        if let Some(target) = self.mic_target.and_then(|id| self.session(id)) {
            let (codec, quality) = target.own_codec();
            self.engine.set_codec(codec, quality);
        }
    }

    fn apply_mute(&mut self, w: &Windows) {
        let (mic, sound) = (self.mic_muted, self.sound_muted);
        let stop_talk = (mic || sound) && self.engine.is_transmitting();
        let target = self
            .mic_target
            .and_then(|id| self.session(id))
            .and_then(|s| s.client.clone().map(|client| (client, s.own_codec().0)));
        let clients: Vec<ClientHandle> =
            self.sessions.iter().filter(|s| s.is_connected()).filter_map(|s| s.client.clone()).collect();
        let engine = &self.engine;
        engine.pause_transmit(|| {
            engine.set_mic_muted(mic);
            engine.set_speaker_muted(sound);
            if let (true, Some((client, codec))) = (stop_talk, &target) {
                client.send_voice(*codec, &[]);
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
    }

    pub fn toggle_sound(&mut self, w: &Windows) {
        self.sound_muted = !self.sound_muted;
        self.apply_mute(w);
    }

    pub fn row_activated(&mut self, w: &Windows, row: TreeRow) {
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
            w.main.set_prompt_name(node.channel.name.as_str().into());
            w.main.set_prompt_password("".into());
            w.main.set_prompt_open(true);
            self.prompt = Some(prompt);
        } else {
            client.join_channel(node.channel.id, "");
        }
    }

    pub fn join_with_password(&mut self, w: &Windows) {
        let password = w.main.get_prompt_password().to_string();
        w.main.set_prompt_open(false);
        w.main.set_prompt_password("".into());
        let Some((session_id, channel)) = self.prompt.take() else {
            return;
        };
        if let Some(client) = self.session(session_id).and_then(|s| s.client.as_ref()) {
            client.join_channel(channel, &password);
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
        let to_server = w.main.get_chat_open() && w.main.get_chat_target() == 1;
        client.send_text(if to_server { TextTarget::Server } else { TextTarget::Channel }, &text);
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
            if w.main.get_dialog_open() {
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
        for client in &outcome.forget {
            self.engine.remove_talker(id, *client);
        }
        if outcome.forget_all {
            self.engine.clear_session(id);
        }
        self.dirty.tree |= viewed && (outcome.tree || outcome.talking.is_some());
        self.dirty.chat |= viewed && outcome.chat;
        self.dirty.sessions |= outcome.header || outcome.connected;
        if outcome.channel && self.mic_target == Some(id) {
            self.route_mic();
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
        if let Some(reason) = outcome.closed {
            self.finish_session(w, id, &reason, was_connecting);
        }
    }

    fn on_connected(&mut self, w: &Windows, id: u16) {
        let Some(session) = self.session_mut(id) else {
            return;
        };
        let save = std::mem::take(&mut session.request.save_bookmark);
        let request = session.request.clone();
        let name = session.name.clone();
        if let (true, Some(client)) = (self.mic_muted || self.sound_muted, self.session(id).and_then(|s| s.client.clone()))
        {
            client.set_mute_state(self.mic_muted, self.sound_muted);
        }
        if save {
            let kept = self.bookmarks.find_address(&request.address).map(|i| self.bookmarks.items[i].name.clone());
            self.bookmarks.upsert(Bookmark {
                name: kept.unwrap_or(name),
                address: request.address.clone(),
                nickname: request.nickname.clone(),
                identity_uid: request.identity_uid.clone(),
            });
            self.save_bookmarks(w);
        }
        self.route_mic();
        self.dirty = Dirty::everything();
    }

    fn status_text(&self) -> String {
        let Some(session) = self.viewed_session() else {
            return "Not connected".to_string();
        };
        if !session.is_connected() {
            return if session.waiting_level.is_some() { session.state_text.clone() } else { "Connecting".to_string() };
        }
        if self.sound_muted {
            return "Sound muted".to_string();
        }
        if self.mic_muted {
            return "Microphone muted".to_string();
        }
        if self.own_talking {
            return "Talking".to_string();
        }
        match self.settings.tx_mode {
            1 => match platform::PTT_KEYS.get(self.settings.ptt_key.max(0) as usize) {
                Some((name, code)) if *code != 0 => format!("Sends while I hold {name}"),
                _ => "Choose a talk key in settings".to_string(),
            },
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
        self.poll_level_jobs(w);

        let key = platform::PTT_KEYS.get(self.settings.ptt_key.max(0) as usize).map(|k| k.1).unwrap_or(0);
        self.engine.set_ptt(platform::key_down(key));

        let level = self.engine.shared().input_level();
        let position = level_position(level);
        let transmitting = self.engine.is_transmitting();
        let on_air = transmitting && self.mic_target.is_some();
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

        let status = self.status_text();
        if status != self.shown_status {
            w.main.set_status_text(status.as_str().into());
            self.shown_status = status;
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
                w.main.set_notice(
                    "Someone is talking with an old voice format (Speex or CELT) that PhishSpeak cannot play.".into(),
                );
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
        let tiles: Vec<ServerTile> = self
            .sessions
            .iter()
            .map(|s| ServerTile {
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
            })
            .collect();
        let others: Vec<ServerTile> = tiles.iter().filter(|t| !t.viewed).take(HEADER_TILES).cloned().collect();
        w.main.set_sessions(ModelRc::new(VecModel::from(tiles)));
        w.main.set_others(ModelRc::new(VecModel::from(others)));
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
        w.main.set_initials(initials(&nickname).into());
        w.main.set_nickname(nickname.into());
        w.main.set_menu_bookmarks(ModelRc::new(VecModel::from(self.bookmark_rows(true))));
    }

    fn publish_bookmarks(&mut self, w: &Windows) {
        let rows = self.bookmark_rows(false);
        w.main.set_bookmarks(ModelRc::new(VecModel::from(rows.clone())));
        w.settings.set_bookmarks(ModelRc::new(VecModel::from(rows)));
        w.main.set_menu_bookmarks(ModelRc::new(VecModel::from(self.bookmark_rows(true))));
    }

    fn publish_tree(&mut self) {
        let rows: Vec<TreeRow> = match self.viewed_session().and_then(|s| s.view.as_ref()) {
            Some(view) => build_rows(view, self.own_talking).iter().map(tree_row).collect(),
            None => Vec::new(),
        };
        sync_rows(&self.tree, rows);
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
        }
        if dirty.chat {
            self.publish_chat(w);
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
