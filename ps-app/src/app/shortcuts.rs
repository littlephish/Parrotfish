use std::time::{Duration, Instant};

use ps_client::spacer::parse_spacer;
use ps_client::{Group, ServerView, WhisperScope, CODEC_OPUS_VOICE};
use slint::{ComponentHandle, SharedString};

use super::{sync_rows, App, Windows};
use crate::hotkeys::{chord_name, Bindings, CaptureStep, Chord, MAX_WHISPER_KEYS, REPLY_LANE};
use crate::platform;
use crate::whisper::{
    describe, lane_table, phrase, resolve, route, Aim, Route, Unusable, WhisperKey, Who, MAX_LIST_CHANNELS,
    MAX_LIST_PEOPLE,
};
use crate::{PickRow, WhisperKeyRow};

pub(super) const MAX_TALK_KEYS: usize = 4;
const SHORTCUTS_TAB: i32 = 4;
const RELEASE_STEP_MS: f32 = 50.0;
const UNHEARD_SHOWN_FOR: Duration = Duration::from_secs(2);
const UNHEARD_IS_FRESH_FOR: Duration = Duration::from_millis(300);
const MAX_PICK_DEPTH: u32 = 12;
const NOTE_TALK: i32 = 0;
const NOTE_WHISPER: i32 = 1;
const NOTE_REPLY: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureTarget {
    Talk(usize),
    NewTalk,
    Reply,
    Whisper(usize),
    EditorKey,
}

impl CaptureTarget {
    pub(super) fn code(self) -> i32 {
        match self {
            CaptureTarget::Talk(index) => index as i32,
            CaptureTarget::NewTalk => 50,
            CaptureTarget::Reply => 60,
            CaptureTarget::EditorKey => 70,
            CaptureTarget::Whisper(index) => 100 + index as i32,
        }
    }

    fn place(self) -> i32 {
        match self {
            CaptureTarget::Talk(_) | CaptureTarget::NewTalk => NOTE_TALK,
            CaptureTarget::Whisper(_) | CaptureTarget::EditorKey => NOTE_WHISPER,
            CaptureTarget::Reply => NOTE_REPLY,
        }
    }
}

pub(super) fn release_text(ms: u32) -> String {
    if ms == 0 {
        return "Off".to_string();
    }
    let seconds = format!("{:.2}", f64::from(ms) / 1000.0);
    format!("{} s", seconds.trim_end_matches('0').trim_end_matches('.'))
}

pub(super) fn talk_hint(names: &[String]) -> String {
    match names {
        [] => "No talk key is set yet. Choose one under Shortcuts.".to_string(),
        [one] => format!("Hold {one} to talk. Change the key under Shortcuts."),
        [one, two] => format!("Hold {one} or {two} to talk. Change the keys under Shortcuts."),
        more => format!("Hold one of your {} talk keys to talk. Change them under Shortcuts.", more.len()),
    }
}

fn scope_from_index(index: i32) -> WhisperScope {
    match index {
        1 => WhisperScope::CurrentChannel,
        2 => WhisperScope::ParentChannel,
        3 => WhisperScope::AllParentChannels,
        4 => WhisperScope::ChannelFamily,
        5 => WhisperScope::WholeFamily,
        6 => WhisperScope::Subchannels,
        _ => WhisperScope::AllChannels,
    }
}

fn shown(name: &str, fallback: &str) -> SharedString {
    if name.trim().is_empty() {
        fallback.into()
    } else {
        name.trim().into()
    }
}

pub(super) struct Here<'a> {
    pub view: &'a ServerView,
    pub name: &'a str,
    pub server_groups: &'a [Group],
    pub channel_groups: &'a [Group],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GroupPick {
    id: u64,
    name: String,
    server_uid: String,
    server_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PickEntry {
    Channel(u64, String),
    Person(String, String),
}

#[derive(Debug, Default)]
pub(super) struct Editor {
    index: Option<usize>,
    chord: Chord,
    kind: i32,
    list_server: (String, String),
    channels: Vec<(u64, String)>,
    people: Vec<(String, String)>,
    who: i32,
    scope: i32,
    server_group: Option<GroupPick>,
    channel_group: Option<GroupPick>,
    entries: Vec<PickEntry>,
    group_options: Vec<GroupPick>,
    note: String,
}

impl Editor {
    fn open(index: Option<usize>, key: Option<&WhisperKey>) -> Self {
        let mut editor = Editor { index, ..Editor::default() };
        let Some(key) = key else {
            return editor;
        };
        editor.chord = key.chord.clone();
        let pick = |id: u64, name: &str| GroupPick {
            id,
            name: name.to_string(),
            server_uid: key.server_uid.clone(),
            server_name: key.server_name.clone(),
        };
        match &key.aim {
            Aim::List { channels, people } => {
                editor.list_server = (key.server_uid.clone(), key.server_name.clone());
                editor.channels = channels.clone();
                editor.people = people.clone();
            }
            Aim::Group { who, scope } => {
                editor.kind = 1;
                editor.scope = *scope as i32;
                match who {
                    Who::Everyone => editor.who = 0,
                    Who::Commanders => editor.who = 1,
                    Who::ServerGroup { id, name } => {
                        editor.who = 2;
                        editor.server_group = Some(pick(*id, name));
                    }
                    Who::ChannelGroup { id, name } => {
                        editor.who = 3;
                        editor.channel_group = Some(pick(*id, name));
                    }
                }
            }
        }
        editor
    }

    fn list_is_empty(&self) -> bool {
        self.channels.is_empty() && self.people.is_empty()
    }

    fn is_local(&self, here: Option<&Here>) -> bool {
        here.is_some_and(|here| {
            !here.view.server.uid.is_empty() && (self.list_is_empty() || self.list_server.0 == here.view.server.uid)
        })
    }

    fn hint(&self, here: Option<&Here>) -> String {
        if let (true, Some(here)) = (self.is_local(here), here) {
            return format!("From {}. Tick the channels and the people your voice should go to.", here.name);
        }
        if self.list_is_empty() {
            return "Connect to a server to choose channels and people from it.".to_string();
        }
        let owner = if self.list_server.1.trim().is_empty() { "another server" } else { self.list_server.1.trim() };
        if here.is_some() {
            format!("This key is for {owner}. View that server to add to it, or untick everything to start again here.")
        } else {
            format!("This key is for {owner}. Connect to that server to add channels and people.")
        }
    }

    fn rows(&mut self, here: Option<&Here>) -> Vec<PickRow> {
        let view = here.filter(|_| self.is_local(here)).map(|here| here.view);
        let mut rows = Vec::new();
        let mut entries = Vec::new();
        for (id, name) in &self.channels {
            let present = view.is_some_and(|v| v.channels.iter().any(|node| node.channel.id == *id));
            if !present {
                rows.push(PickRow { person: false, depth: 0, text: shown(name, "A channel"), ticked: true, gone: true });
                entries.push(PickEntry::Channel(*id, name.clone()));
            }
        }
        for (uid, name) in &self.people {
            let present = view.is_some_and(|v| {
                v.channels.iter().flat_map(|node| node.clients.iter()).any(|c| c.uid == *uid && c.id != v.own_id)
            });
            if !present {
                rows.push(PickRow { person: true, depth: 0, text: shown(name, "Someone"), ticked: true, gone: true });
                entries.push(PickEntry::Person(uid.clone(), name.clone()));
            }
        }
        if let Some(view) = view {
            for node in &view.channels {
                if parse_spacer(&node.channel.name, node.channel.parent).is_some() {
                    continue;
                }
                let depth = node.depth.min(MAX_PICK_DEPTH) as i32;
                rows.push(PickRow {
                    person: false,
                    depth,
                    text: node.channel.name.as_str().into(),
                    ticked: self.channels.iter().any(|(id, _)| *id == node.channel.id),
                    gone: false,
                });
                entries.push(PickEntry::Channel(node.channel.id, node.channel.name.clone()));
                for client in &node.clients {
                    if client.id == view.own_id || client.is_query || client.uid.is_empty() {
                        continue;
                    }
                    rows.push(PickRow {
                        person: true,
                        depth: depth + 1,
                        text: client.nickname.as_str().into(),
                        ticked: self.people.iter().any(|(uid, _)| *uid == client.uid),
                        gone: false,
                    });
                    entries.push(PickEntry::Person(client.uid.clone(), client.nickname.clone()));
                }
            }
        }
        self.entries = entries;
        rows
    }

    fn adopt(&mut self, here: Option<&Here>) -> bool {
        let Some(here) = here else {
            return false;
        };
        if here.view.server.uid.is_empty() {
            self.note = "This server has not said who it is yet. Try again in a moment.".to_string();
            return false;
        }
        if self.list_is_empty() {
            self.list_server = (here.view.server.uid.clone(), here.name.to_string());
        }
        self.list_server.0 == here.view.server.uid
    }

    fn toggle(&mut self, index: usize, here: Option<&Here>) {
        self.note.clear();
        let Some(entry) = self.entries.get(index).cloned() else {
            return;
        };
        match entry {
            PickEntry::Channel(id, name) => {
                if let Some(at) = self.channels.iter().position(|(have, _)| *have == id) {
                    self.channels.remove(at);
                } else if self.channels.len() >= MAX_LIST_CHANNELS {
                    self.note = format!("A whisper key can hold {MAX_LIST_CHANNELS} channels.");
                } else if self.adopt(here) {
                    self.channels.push((id, name));
                }
            }
            PickEntry::Person(uid, name) => {
                if let Some(at) = self.people.iter().position(|(have, _)| *have == uid) {
                    self.people.remove(at);
                } else if self.people.len() >= MAX_LIST_PEOPLE {
                    self.note = format!("A whisper key can hold {MAX_LIST_PEOPLE} people.");
                } else if self.adopt(here) {
                    self.people.push((uid, name));
                }
            }
        }
        if self.list_is_empty() {
            self.list_server = (String::new(), String::new());
        }
    }

    fn groups(&mut self, here: Option<&Here>) -> (Vec<SharedString>, i32) {
        let current = match self.who {
            2 => self.server_group.clone(),
            3 => self.channel_group.clone(),
            _ => {
                self.group_options.clear();
                return (Vec::new(), -1);
            }
        };
        let mut options: Vec<GroupPick> = Vec::new();
        let mut names: Vec<SharedString> = Vec::new();
        if let Some(here) = here.filter(|here| !here.view.server.uid.is_empty()) {
            let source = if self.who == 2 { here.server_groups } else { here.channel_groups };
            for group in source {
                options.push(GroupPick {
                    id: group.id,
                    name: group.name.clone(),
                    server_uid: here.view.server.uid.clone(),
                    server_name: here.name.to_string(),
                });
                names.push(shown(&group.name, "A group"));
            }
        }
        let mut selected = -1;
        if let Some(pick) = current {
            match options.iter().position(|option| option.id == pick.id && option.server_uid == pick.server_uid) {
                Some(at) => selected = at as i32,
                None => {
                    let name = shown(&pick.name, "A group");
                    let label = if pick.server_name.trim().is_empty() {
                        format!("{name} (not on this server)")
                    } else {
                        format!("{name}, on {}", pick.server_name.trim())
                    };
                    options.insert(0, pick);
                    names.insert(0, label.into());
                    selected = 0;
                }
            }
        }
        self.group_options = options;
        (names, selected)
    }

    fn choose_group(&mut self, index: i32) {
        let Some(pick) = usize::try_from(index).ok().and_then(|at| self.group_options.get(at).cloned()) else {
            return;
        };
        match self.who {
            2 => self.server_group = Some(pick),
            3 => self.channel_group = Some(pick),
            _ => {}
        }
    }

    fn refresh_names(&mut self, here: Option<&Here>) {
        let Some(here) = here.filter(|_| self.is_local(here)) else {
            return;
        };
        for (id, name) in &mut self.channels {
            if let Some(node) = here.view.channels.iter().find(|node| node.channel.id == *id) {
                *name = node.channel.name.clone();
            }
        }
        for (uid, name) in &mut self.people {
            let found = here.view.channels.iter().flat_map(|node| node.clients.iter()).find(|c| c.uid == *uid);
            if let Some(client) = found {
                *name = client.nickname.clone();
            }
        }
        if !self.list_is_empty() {
            self.list_server.1 = here.name.to_string();
        }
    }

    fn key(&self) -> Option<WhisperKey> {
        let bound = |pick: &GroupPick| (!pick.server_uid.is_empty()).then(|| (pick.server_uid.clone(), pick.server_name.clone()));
        let (aim, server) = if self.kind == 0 {
            if self.list_is_empty() || self.list_server.0.is_empty() {
                return None;
            }
            (Aim::List { channels: self.channels.clone(), people: self.people.clone() }, self.list_server.clone())
        } else {
            let scope = scope_from_index(self.scope);
            match self.who {
                0 => (Aim::Group { who: Who::Everyone, scope }, (String::new(), String::new())),
                1 => (Aim::Group { who: Who::Commanders, scope }, (String::new(), String::new())),
                2 => {
                    let pick = self.server_group.as_ref()?;
                    (Aim::Group { who: Who::ServerGroup { id: pick.id, name: pick.name.clone() }, scope }, bound(pick)?)
                }
                3 => {
                    let pick = self.channel_group.as_ref()?;
                    (Aim::Group { who: Who::ChannelGroup { id: pick.id, name: pick.name.clone() }, scope }, bound(pick)?)
                }
                _ => return None,
            }
        };
        Some(WhisperKey { chord: self.chord.clone(), server_uid: server.0, server_name: server.1, aim })
    }
}

impl App {
    fn here(&self) -> Option<Here<'_>> {
        let session = self.viewed_session()?;
        if !session.is_connected() {
            return None;
        }
        Some(Here {
            view: session.view.as_ref()?,
            name: &session.name,
            server_groups: &session.server_groups,
            channel_groups: &session.channel_groups,
        })
    }

    pub(super) fn push_bindings(&self) {
        let bindings = Bindings {
            talk: self.settings.talk_keys.clone(),
            whisper: self.whisper_keys.items.iter().map(|key| key.chord.clone()).collect(),
            reply: self.settings.reply_key.clone(),
        };
        self.watcher.state().set_bindings(bindings);
        self.watcher.state().set_release_delay(self.settings.talk_release_ms);
    }

    pub(super) fn rebuild_lanes(&mut self, fresh: bool) {
        let shared = self.engine.shared().clone();
        let busy = !fresh && (shared.whisper_lane() == REPLY_LANE || shared.on_air_lane() == Some(REPLY_LANE));
        let session = self.mic_target.and_then(|id| self.session(id));
        let view = session.and_then(|s| s.view.as_ref());
        let latest = session.and_then(|s| s.reply_to.clone().map(|(client, uid)| (s.id, client, uid)));
        let live = if busy { self.reply_live.clone() } else { latest.clone() };
        let reply = match (view, &live) {
            (Some(view), Some((owner, client, uid)))
                if Some(*owner) == self.mic_target && view.client(*client).is_some_and(|c| c.uid == *uid) =>
            {
                Some(*client)
            }
            _ => None,
        };
        let table = lane_table(&self.whisper_keys.items, view, reply);
        let mut names = vec![String::new(); table.len()];
        let mut notes = vec![String::new(); table.len()];
        for (index, key) in self.whisper_keys.items.iter().enumerate().take(MAX_WHISPER_KEYS) {
            names[index + 1] = phrase(&key.aim);
            notes[index + 1] = match view.map(|view| resolve(key, view)) {
                Some(Err(Unusable::OtherServer)) if key.server_name.trim().is_empty() => {
                    "That whisper key is for another server".to_string()
                }
                Some(Err(Unusable::OtherServer)) => format!("That whisper key is for {}", key.server_name.trim()),
                Some(Err(Unusable::NobodyThere)) => "Nobody on that whisper key is here".to_string(),
                _ => String::new(),
            };
        }
        let reply_lane = usize::from(REPLY_LANE);
        names[reply_lane] =
            reply.and_then(|id| view.and_then(|v| v.client(id))).map(|c| c.nickname.clone()).unwrap_or_default();
        notes[reply_lane] = if live.is_some() {
            "The person who whispered to you has left".to_string()
        } else {
            "Nobody has whispered to you yet".to_string()
        };
        for (lane, target) in table.iter().enumerate() {
            if let Some(target) = target {
                shared.set_lane_room(lane as u8, target.frame_room());
            }
        }
        self.lanes_stale = busy && live != latest;
        self.reply_live = live;
        self.lane_names = names;
        self.lane_notes = notes;
        if let Ok(mut slot) = self.lanes.lock() {
            *slot = table;
        }
    }

    fn lane_is_empty(&self, lane: u8) -> bool {
        self.lanes.lock().map(|table| route(lane, &table) == Route::Nothing).unwrap_or(true)
    }

    pub(super) fn whisper_goes_nowhere(&self) -> bool {
        self.engine.shared().on_air_lane().is_some_and(|lane| {
            lane != 0 && (self.lane_is_empty(lane) || self.unheard.is_some_and(|(silent, _)| silent == lane))
        })
    }

    pub(super) fn whisper_status(&self) -> Option<String> {
        let shared = self.engine.shared();
        let held = shared.whisper_lane();
        if held != 0 && self.lane_is_empty(held) {
            let note = self.lane_notes.get(usize::from(held)).filter(|note| !note.is_empty());
            return Some(note.cloned().unwrap_or_else(|| "That whisper key has nobody to reach".to_string()));
        }
        let unheard = "Nobody is there to hear that whisper";
        match shared.on_air_lane() {
            Some(lane) if lane != 0 && !self.lane_is_empty(lane) => {
                if self.unheard.is_some_and(|(silent, _)| silent == lane) {
                    return Some(unheard.to_string());
                }
                match self.lane_names.get(usize::from(lane)).filter(|name| !name.is_empty()) {
                    Some(name) => Some(format!("Whispering to {name}")),
                    None => Some("Whispering".to_string()),
                }
            }
            _ => self.unheard.filter(|(_, at)| at.elapsed() < UNHEARD_SHOWN_FOR).map(|_| unheard.to_string()),
        }
    }

    pub(super) fn note_unheard(&mut self) {
        let lane = self.engine.shared().on_air_lane().filter(|lane| *lane != 0).unwrap_or(self.last_whisper_lane);
        if lane != 0 {
            self.unheard = Some((lane, Instant::now()));
        }
    }

    pub(super) fn watch_whispers(&mut self) {
        let shared = self.engine.shared().clone();
        let air = shared.on_air_lane().filter(|lane| *lane != 0);
        if air != self.whisper_air {
            if air.is_some() && self.unheard.is_some_and(|(_, at)| at.elapsed() > UNHEARD_IS_FRESH_FOR) {
                self.unheard = None;
            }
            self.whisper_air = air;
        }
        if let Some(lane) = air {
            self.last_whisper_lane = lane;
        } else if self.unheard.is_some_and(|(_, at)| at.elapsed() >= UNHEARD_SHOWN_FOR) {
            self.unheard = None;
        }
        let reply_free = shared.whisper_lane() != REPLY_LANE && shared.on_air_lane() != Some(REPLY_LANE);
        if self.lanes_stale && reply_free {
            self.rebuild_lanes(false);
        }
    }

    fn keys_changed(&mut self) {
        let shared = self.engine.shared().clone();
        let paused = shared.sink.lock();
        if let Some(lane) = shared.on_air_lane().filter(|lane| *lane != 0) {
            let client = self.mic_target.and_then(|id| self.session(id)).and_then(|s| s.client.clone());
            if let (Some(client), Ok(table)) = (client, self.lanes.lock()) {
                if let Route::Whisper(target) = route(lane, &table) {
                    client.send_whisper(target, CODEC_OPUS_VOICE, &[]);
                }
            }
        }
        self.push_bindings();
        self.rebuild_lanes(true);
        drop(paused);
    }

    fn store_whisper_keys(&mut self) {
        if let Err(e) = self.whisper_keys.save() {
            self.shortcut_note = (NOTE_WHISPER, format!("Could not save your whisper keys: {e}."));
        }
    }

    fn clear_notes(&mut self) {
        self.shortcut_note = (NOTE_TALK, String::new());
        if let Some(editor) = &mut self.editor {
            editor.note.clear();
        }
        self.dirty.shortcuts = true;
    }

    fn key_use(&self, chord: &Chord, target: CaptureTarget) -> Option<&'static str> {
        let mut talk = self.settings.talk_keys.iter().enumerate();
        if talk.any(|(index, key)| key == chord && target != CaptureTarget::Talk(index)) {
            return Some("a talk key");
        }
        if self.settings.reply_key == *chord && target != CaptureTarget::Reply {
            return Some("the reply key");
        }
        let own = match target {
            CaptureTarget::Whisper(index) => Some(index),
            CaptureTarget::EditorKey => self.editor.as_ref().and_then(|editor| editor.index),
            _ => None,
        };
        let mut whisper = self.whisper_keys.items.iter().enumerate();
        if whisper.any(|(index, key)| key.chord == *chord && Some(index) != own) {
            let mine = matches!(target, CaptureTarget::Whisper(_) | CaptureTarget::EditorKey);
            return Some(if mine { "another whisper key" } else { "a whisper key" });
        }
        None
    }

    fn assign(&mut self, target: CaptureTarget, chord: Chord) {
        if let Some(used) = self.key_use(&chord, target) {
            let talk = matches!(target, CaptureTarget::Talk(_) | CaptureTarget::NewTalk);
            let text = if talk && used == "a talk key" {
                "That key is already a talk key.".to_string()
            } else {
                format!("That key is already used for {used}.")
            };
            match (&mut self.editor, target) {
                (Some(editor), CaptureTarget::EditorKey) => editor.note = text,
                _ => self.shortcut_note = (target.place(), text),
            }
            return;
        }
        match target {
            CaptureTarget::Talk(index) => {
                if let Some(slot) = self.settings.talk_keys.get_mut(index) {
                    *slot = chord;
                }
            }
            CaptureTarget::NewTalk => {
                if self.settings.talk_keys.len() < MAX_TALK_KEYS {
                    self.settings.talk_keys.push(chord);
                }
            }
            CaptureTarget::Reply => self.settings.reply_key = chord,
            CaptureTarget::Whisper(index) => {
                if let Some(key) = self.whisper_keys.items.get_mut(index) {
                    key.chord = chord;
                }
                self.store_whisper_keys();
            }
            CaptureTarget::EditorKey => {
                if let Some(editor) = &mut self.editor {
                    editor.chord = chord;
                }
                return;
            }
        }
        self.mark_settings_dirty();
        self.keys_changed();
    }

    fn begin_capture(&mut self, target: CaptureTarget) {
        let again = self.capture == Some(target);
        self.stop_capture();
        self.clear_notes();
        if !again {
            self.watcher.state().begin_capture();
            self.capture = Some(target);
        }
    }

    pub(super) fn stop_capture(&mut self) {
        if self.capture.take().is_some() {
            self.watcher.state().cancel_capture();
            self.dirty.shortcuts = true;
        }
    }

    pub(super) fn poll_capture(&mut self, w: &Windows) {
        let Some(target) = self.capture else {
            return;
        };
        if !w.settings.window().is_visible() || w.settings.get_tab() != SHORTCUTS_TAB {
            self.stop_capture();
            return;
        }
        match self.watcher.state().take_captured() {
            Some(CaptureStep::Done(chord)) => {
                self.capture = None;
                self.assign(target, chord);
                self.dirty.shortcuts = true;
            }
            Some(CaptureStep::Cancelled) => {
                self.capture = None;
                self.dirty.shortcuts = true;
            }
            Some(CaptureStep::Waiting) | None => {}
        }
    }

    pub(super) fn close_editor(&mut self) {
        self.stop_capture();
        if self.editor.take().is_some() {
            self.dirty.shortcuts = true;
        }
    }

    pub fn talk_key_change(&mut self, _w: &Windows, index: i32) {
        if let Some(index) = usize::try_from(index).ok().filter(|index| *index < self.settings.talk_keys.len()) {
            self.begin_capture(CaptureTarget::Talk(index));
        }
    }

    pub fn talk_key_add(&mut self, _w: &Windows) {
        if self.settings.talk_keys.len() < MAX_TALK_KEYS {
            self.begin_capture(CaptureTarget::NewTalk);
        }
    }

    pub fn talk_key_remove(&mut self, _w: &Windows, index: i32) {
        self.stop_capture();
        self.clear_notes();
        if let Some(index) = usize::try_from(index).ok().filter(|index| *index < self.settings.talk_keys.len()) {
            self.settings.talk_keys.remove(index);
            self.mark_settings_dirty();
            self.keys_changed();
        }
    }

    pub fn reply_key_change(&mut self, _w: &Windows) {
        self.begin_capture(CaptureTarget::Reply);
    }

    pub fn reply_key_clear(&mut self, _w: &Windows) {
        self.stop_capture();
        self.clear_notes();
        self.settings.reply_key = Chord::default();
        self.mark_settings_dirty();
        self.keys_changed();
    }

    pub fn shortcut_changed(&mut self, w: &Windows) {
        let steps = (w.settings.get_talk_release() / RELEASE_STEP_MS).round().clamp(0.0, 20.0);
        let release = steps as u32 * RELEASE_STEP_MS as u32;
        if release != self.settings.talk_release_ms {
            self.settings.talk_release_ms = release;
            self.watcher.state().set_release_delay(release);
            self.mark_settings_dirty();
        }
        let allow = w.settings.get_allow_whispers();
        if allow != self.settings.allow_whispers {
            self.settings.allow_whispers = allow;
            self.allow_whispers.store(allow, std::sync::atomic::Ordering::Relaxed);
            for session in &mut self.sessions {
                session.set_allow_whispers(allow);
            }
            self.mark_settings_dirty();
            self.dirty.tree = true;
            self.dirty.sessions = true;
        }
        self.dirty.shortcuts = true;
    }

    pub fn whisper_key_change(&mut self, _w: &Windows, index: i32) {
        if let Some(index) = usize::try_from(index).ok().filter(|index| *index < self.whisper_keys.items.len()) {
            self.begin_capture(CaptureTarget::Whisper(index));
        }
    }

    pub fn whisper_key_add(&mut self, _w: &Windows) {
        self.stop_capture();
        self.clear_notes();
        if self.whisper_keys.items.len() < MAX_WHISPER_KEYS {
            self.editor = Some(Editor::open(None, None));
        }
    }

    pub fn whisper_key_edit(&mut self, _w: &Windows, index: i32) {
        self.stop_capture();
        self.clear_notes();
        let Some(index) = usize::try_from(index).ok().filter(|index| *index < self.whisper_keys.items.len()) else {
            return;
        };
        self.editor = Some(Editor::open(Some(index), self.whisper_keys.items.get(index)));
    }

    pub fn whisper_key_remove(&mut self, _w: &Windows, index: i32) {
        self.stop_capture();
        self.clear_notes();
        if let Some(index) = usize::try_from(index).ok().filter(|index| *index < self.whisper_keys.items.len()) {
            self.whisper_keys.items.remove(index);
            self.store_whisper_keys();
            self.keys_changed();
        }
    }

    pub fn editor_key_change(&mut self, _w: &Windows) {
        if self.editor.is_some() {
            self.begin_capture(CaptureTarget::EditorKey);
        }
    }

    pub fn editor_key_clear(&mut self, _w: &Windows) {
        self.stop_capture();
        self.clear_notes();
        if let Some(editor) = &mut self.editor {
            editor.chord = Chord::default();
        }
    }

    pub fn editor_toggle(&mut self, _w: &Windows, index: i32) {
        self.stop_capture();
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        editor.toggle(index, self.here().as_ref());
        self.editor = Some(editor);
        self.dirty.shortcuts = true;
    }

    pub fn editor_changed(&mut self, w: &Windows) {
        self.stop_capture();
        let Some(editor) = &mut self.editor else {
            return;
        };
        editor.note.clear();
        editor.kind = w.settings.get_editor_kind().clamp(0, 1);
        editor.scope = w.settings.get_editor_scope().clamp(0, 6);
        let who = w.settings.get_editor_who().clamp(0, 3);
        if who == editor.who {
            editor.choose_group(w.settings.get_editor_group());
        } else {
            editor.who = who;
        }
        self.dirty.shortcuts = true;
    }

    pub fn editor_save(&mut self, _w: &Windows) {
        self.stop_capture();
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        editor.refresh_names(self.here().as_ref());
        self.dirty.shortcuts = true;
        let Some(key) = editor.key() else {
            editor.note = "Choose who this key whispers to first.".to_string();
            self.editor = Some(editor);
            return;
        };
        match editor.index {
            Some(index) if index < self.whisper_keys.items.len() => self.whisper_keys.items[index] = key,
            _ if self.whisper_keys.items.len() < MAX_WHISPER_KEYS => self.whisper_keys.items.push(key),
            _ => {}
        }
        self.shortcut_note = (NOTE_WHISPER, String::new());
        self.store_whisper_keys();
        self.keys_changed();
    }

    pub fn editor_cancel(&mut self, _w: &Windows) {
        self.close_editor();
    }

    pub(super) fn publish_shortcuts(&mut self, w: &Windows) {
        let name = |chord: &Chord| chord_name(chord, &platform::key_char);
        let talk: Vec<String> = self.settings.talk_keys.iter().map(name).collect();
        w.settings.set_talk_hint(talk_hint(&talk).into());
        sync_rows(&self.talk_rows, talk.into_iter().map(SharedString::from).collect());
        w.settings.set_can_add_talk(self.settings.talk_keys.len() < MAX_TALK_KEYS);
        w.settings.set_listening(self.capture.map_or(-1, CaptureTarget::code));
        w.settings.set_talk_release(self.settings.talk_release_ms as f32);
        w.settings.set_talk_release_text(release_text(self.settings.talk_release_ms).into());
        w.settings.set_shortcut_note(self.shortcut_note.1.as_str().into());
        w.settings.set_shortcut_note_place(self.shortcut_note.0);
        let keys: Vec<WhisperKeyRow> = self
            .whisper_keys
            .items
            .iter()
            .map(|key| WhisperKeyRow {
                key: name(&key.chord).into(),
                summary: describe(&key.aim).into(),
                note: if key.server_uid.is_empty() {
                    SharedString::new()
                } else if key.server_name.trim().is_empty() {
                    "for one server".into()
                } else {
                    format!("for {}", key.server_name.trim()).into()
                },
            })
            .collect();
        sync_rows(&self.whisper_rows, keys);
        w.settings.set_can_add_whisper(self.whisper_keys.items.len() < MAX_WHISPER_KEYS);
        w.settings.set_reply_key(name(&self.settings.reply_key).into());
        w.settings.set_allow_whispers(self.settings.allow_whispers);
        w.settings.set_fold_mode(self.settings.fold_mode);

        let Some(mut editor) = self.editor.take() else {
            w.settings.set_editor_open(false);
            sync_rows(&self.pick_rows, Vec::new());
            sync_rows(&self.group_rows, Vec::new());
            return;
        };
        let here = self.here();
        let rows = editor.rows(here.as_ref());
        let hint = editor.hint(here.as_ref());
        let (groups, group) = editor.groups(here.as_ref());
        drop(here);
        sync_rows(&self.pick_rows, rows);
        sync_rows(&self.group_rows, groups);
        w.settings.set_editor_open(true);
        w.settings.set_editor_title(if editor.index.is_some() { "Edit whisper key" } else { "New whisper key" }.into());
        w.settings.set_editor_key(name(&editor.chord).into());
        w.settings.set_editor_kind(editor.kind);
        w.settings.set_editor_hint(hint.into());
        w.settings.set_editor_who(editor.who);
        w.settings.set_editor_group(group);
        w.settings.set_editor_scope(editor.scope);
        w.settings.set_editor_note(editor.note.as_str().into());
        w.settings.set_editor_can_save(editor.key().is_some());
        self.editor = Some(editor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ps_client::{Channel, ChannelNode, ClientInfo, ServerInfo};

    fn view(uid: &str) -> ServerView {
        let person = |id: u16, name: &str, uid: &str| ClientInfo {
            id,
            nickname: name.into(),
            uid: uid.into(),
            ..ClientInfo::default()
        };
        let node = |id: u64, parent: u64, depth: u32, name: &str, clients: Vec<ClientInfo>| ChannelNode {
            channel: Channel { id, parent, name: name.into(), ..Channel::default() },
            depth,
            clients,
        };
        let mut query = person(30, "serveradmin", "uidQuery");
        query.is_query = true;
        ServerView {
            server: ServerInfo { name: "Reef Runners".into(), uid: uid.into(), ..ServerInfo::default() },
            own_id: 7,
            own_channel: 1,
            channels: vec![
                node(9, 0, 0, "[cspacer]Welcome", vec![]),
                node(1, 0, 0, "Lobby", vec![person(7, "Minnow", "uidMe"), person(8, "Marlin", "uidMarlin"), query]),
                node(2, 0, 0, "Deep Rock", vec![]),
                node(4, 2, 1, "Radio", vec![person(9, "Coralline", "uidCora")]),
            ],
        }
    }

    fn groups() -> (Vec<Group>, Vec<Group>) {
        let group = |id: u64, name: &str| Group { id, name: name.into(), kind: 1, sort: 0, icon: 0 };
        (vec![group(6, "Server Admin"), group(8, "Guest")], vec![group(5, "Channel Admin")])
    }

    fn texts(rows: &[PickRow]) -> Vec<(String, bool, bool, i32)> {
        rows.iter().map(|row| (row.text.to_string(), row.ticked, row.gone, row.depth)).collect()
    }

    #[test]
    fn readouts_read_naturally() {
        assert_eq!(release_text(0), "Off");
        assert_eq!(release_text(50), "0.05 s");
        assert_eq!(release_text(200), "0.2 s");
        assert_eq!(release_text(1000), "1 s");
        assert_eq!(talk_hint(&[]), "No talk key is set yet. Choose one under Shortcuts.");
        assert_eq!(talk_hint(&["F8".into()]), "Hold F8 to talk. Change the key under Shortcuts.");
        assert!(talk_hint(&["F8".into(), "Mouse 4".into()]).starts_with("Hold F8 or Mouse 4 to talk."));
        assert!(talk_hint(&["A".into(), "B".into(), "C".into()]).starts_with("Hold one of your 3 talk keys"));
        let codes: Vec<i32> = [
            CaptureTarget::Talk(0),
            CaptureTarget::Talk(3),
            CaptureTarget::NewTalk,
            CaptureTarget::Reply,
            CaptureTarget::EditorKey,
            CaptureTarget::Whisper(0),
            CaptureTarget::Whisper(11),
        ]
        .iter()
        .map(|target| target.code())
        .collect();
        assert_eq!(codes, vec![0, 3, 50, 60, 70, 100, 111]);
        for index in 0..7 {
            assert_eq!(scope_from_index(index) as i32, index);
        }
        assert_eq!(scope_from_index(99), WhisperScope::AllChannels);
    }

    #[test]
    fn a_list_is_ticked_from_the_viewed_server() {
        let reef = view("serverA");
        let (server_groups, channel_groups) = groups();
        let here = Here { view: &reef, name: "Reef Runners", server_groups: &server_groups, channel_groups: &channel_groups };
        let mut editor = Editor::open(None, None);
        assert!(editor.key().is_none());
        assert_eq!(editor.hint(None), "Connect to a server to choose channels and people from it.");
        assert!(editor.rows(None).is_empty());

        let rows = editor.rows(Some(&here));
        assert_eq!(
            texts(&rows),
            vec![
                ("Lobby".to_string(), false, false, 0),
                ("Marlin".to_string(), false, false, 1),
                ("Deep Rock".to_string(), false, false, 0),
                ("Radio".to_string(), false, false, 1),
                ("Coralline".to_string(), false, false, 2),
            ]
        );
        assert!(rows[1].person && !rows[0].person);
        assert!(editor.hint(Some(&here)).starts_with("From Reef Runners."));

        editor.toggle(0, Some(&here));
        editor.toggle(4, Some(&here));
        editor.toggle(99, Some(&here));
        assert_eq!(editor.list_server, ("serverA".to_string(), "Reef Runners".to_string()));
        let rows = editor.rows(Some(&here));
        assert!(rows[0].ticked && rows[4].ticked && !rows[1].ticked);
        let key = editor.key().unwrap();
        assert_eq!(key.server_uid, "serverA");
        assert_eq!(
            key.aim,
            Aim::List { channels: vec![(1, "Lobby".into())], people: vec![("uidCora".into(), "Coralline".into())] }
        );

        editor.toggle(0, Some(&here));
        editor.toggle(4, Some(&here));
        assert!(editor.key().is_none());
        assert_eq!(editor.list_server, (String::new(), String::new()));
    }

    #[test]
    fn entries_that_are_gone_come_first_and_other_servers_stay_apart() {
        let reef = view("serverA");
        let (server_groups, channel_groups) = groups();
        let here = Here { view: &reef, name: "Reef Runners", server_groups: &server_groups, channel_groups: &channel_groups };
        let saved = WhisperKey {
            chord: Chord::new(&[0x65]),
            server_uid: "serverA".into(),
            server_name: "Old name".into(),
            aim: Aim::List {
                channels: vec![(2, "Deep".into()), (40, "Gone".into())],
                people: vec![("uidOffline".into(), "Pike".into()), ("uidMarlin".into(), "Marl".into())],
            },
        };
        let mut editor = Editor::open(Some(3), Some(&saved));
        let rows = editor.rows(Some(&here));
        assert_eq!(
            texts(&rows)[..2],
            [("Gone".to_string(), true, true, 0), ("Pike".to_string(), true, true, 0)]
        );
        assert!(rows[3].ticked && rows[4].ticked && rows.len() == 7);
        editor.toggle(0, Some(&here));
        assert_eq!(editor.rows(Some(&here)).len(), 6);
        editor.refresh_names(Some(&here));
        let key = editor.key().unwrap();
        assert_eq!(key.server_name, "Reef Runners");
        assert_eq!(
            key.aim,
            Aim::List {
                channels: vec![(2, "Deep Rock".into())],
                people: vec![("uidOffline".into(), "Pike".into()), ("uidMarlin".into(), "Marlin".into())],
            }
        );

        let elsewhere = view("serverB");
        let there = Here { view: &elsewhere, name: "Night Shift", server_groups: &[], channel_groups: &[] };
        let mut foreign = Editor::open(Some(3), Some(&saved));
        let rows = foreign.rows(Some(&there));
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|row| row.gone && row.ticked));
        assert!(foreign.hint(Some(&there)).starts_with("This key is for Old name. View that server"));
        assert!(foreign.hint(None).starts_with("This key is for Old name. Connect to that server"));
        for _ in 0..4 {
            foreign.toggle(0, Some(&there));
            foreign.rows(Some(&there));
        }
        assert!(foreign.list_is_empty());
        let rows = foreign.rows(Some(&there));
        assert_eq!(rows.len(), 5);
        foreign.toggle(0, Some(&there));
        assert_eq!(foreign.key().unwrap().server_uid, "serverB");

        let nameless = view("");
        let nowhere = Here { view: &nameless, name: "Mystery", server_groups: &[], channel_groups: &[] };
        let mut blank = Editor::open(None, None);
        assert!(blank.rows(Some(&nowhere)).is_empty());
        assert!(blank.key().is_none());
    }

    #[test]
    fn a_list_stops_at_its_limits() {
        let mut crowded = view("serverA");
        for n in 0..40u64 {
            crowded.channels.push(ChannelNode {
                channel: Channel { id: 100 + n, name: format!("Room {n}"), ..Channel::default() },
                depth: 0,
                clients: (0..2u16)
                    .map(|k| ClientInfo {
                        id: 200 + n as u16 * 2 + k,
                        nickname: format!("P{n}-{k}"),
                        uid: format!("uid{n}-{k}"),
                        ..ClientInfo::default()
                    })
                    .collect(),
            });
        }
        let here = Here { view: &crowded, name: "Reef Runners", server_groups: &[], channel_groups: &[] };
        let mut editor = Editor::open(None, None);
        let rows = editor.rows(Some(&here));
        for index in 0..rows.len() {
            editor.toggle(index, Some(&here));
        }
        assert_eq!((editor.channels.len(), editor.people.len()), (MAX_LIST_CHANNELS, MAX_LIST_PEOPLE));
        assert_eq!(editor.note, format!("A whisper key can hold {MAX_LIST_PEOPLE} people."));
        let key = editor.key().unwrap();
        assert!(resolve(&key, &crowded).unwrap().frame_room() >= 122);
    }

    #[test]
    fn a_group_key_needs_a_group_from_a_server() {
        let reef = view("serverA");
        let (server_groups, channel_groups) = groups();
        let here = Here { view: &reef, name: "Reef Runners", server_groups: &server_groups, channel_groups: &channel_groups };
        let mut editor = Editor::open(None, None);
        editor.kind = 1;
        editor.scope = 2;
        assert_eq!(
            editor.key().unwrap().aim,
            Aim::Group { who: Who::Everyone, scope: WhisperScope::ParentChannel }
        );
        assert_eq!(editor.key().unwrap().server_uid, "");
        assert_eq!(editor.groups(Some(&here)), (Vec::new(), -1));
        editor.who = 1;
        assert_eq!(editor.key().unwrap().aim, Aim::Group { who: Who::Commanders, scope: WhisperScope::ParentChannel });

        editor.who = 2;
        assert!(editor.key().is_none());
        let (names, selected) = editor.groups(Some(&here));
        assert_eq!((names.len(), names[0].as_str(), selected), (2, "Server Admin", -1));
        editor.choose_group(-1);
        assert!(editor.key().is_none());
        editor.choose_group(1);
        let key = editor.key().unwrap();
        assert_eq!((key.server_uid.as_str(), key.server_name.as_str()), ("serverA", "Reef Runners"));
        assert_eq!(
            key.aim,
            Aim::Group { who: Who::ServerGroup { id: 8, name: "Guest".into() }, scope: WhisperScope::ParentChannel }
        );
        assert_eq!(editor.groups(Some(&here)).1, 1);

        editor.who = 3;
        assert!(editor.key().is_none());
        assert_eq!(editor.groups(Some(&here)).0.len(), 1);
        editor.choose_group(0);
        assert!(matches!(editor.key().unwrap().aim, Aim::Group { who: Who::ChannelGroup { id: 5, .. }, .. }));
        assert!(editor.groups(None).0.len() == 1 && editor.groups(None).1 == 0);

        let reopened = Editor::open(Some(0), Some(&key));
        assert_eq!((reopened.kind, reopened.who, reopened.scope), (1, 2, 2));
        let elsewhere = view("serverB");
        let there = Here { view: &elsewhere, name: "Night Shift", server_groups: &server_groups, channel_groups: &[] };
        let mut moved = reopened;
        let (names, selected) = moved.groups(Some(&there));
        assert_eq!((names.len(), names[0].as_str(), selected), (3, "Guest, on Reef Runners", 0));
        assert_eq!(moved.key().unwrap().server_uid, "serverA");
        moved.choose_group(2);
        let rebound = moved.key().unwrap();
        assert_eq!((rebound.server_uid.as_str(), rebound.server_name.as_str()), ("serverB", "Night Shift"));
    }
}
