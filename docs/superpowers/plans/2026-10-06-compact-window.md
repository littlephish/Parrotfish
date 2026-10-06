# Compact Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single-server window with the compact tree window in Twilight reef: rendered spacer channels, bookmarks, several servers connected at once, and all settings in a tabbed settings window.

**Architecture:** `ps-client` learns to recognise spacer channels. `ps-voice` identifies talkers by connection and client so several servers can play at once. `ps-app` is rebuilt around a list of sessions, one per connection, with one viewed session that owns the microphone; the Slint UI is split into a theme, widgets, the main window and a settings window.

**Tech Stack:** Rust stable, Slint 1.18 (fluent widgets for text fields, sliders and combo boxes; custom components for everything themed), cpal, existing workspace crates. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-06-compact-window-design.md`

**How to read this plan:** I will execute it myself in this session, so each step names its files, interfaces and tests exactly but does not repeat every line of code. The code is written test-first where a unit test is possible. The project is not a git repository, so there are no commit steps.

## Global Constraints

- System font only (Segoe UI on Windows). No bundled typeface, no new crates.
- Palette, exact values: Trench `#10222E`, Reef `#18313F`, Shoal `#1F3D4E`, Current `#26495E`, Foam `#E9F2F1`, Drift `#8DA9B3`, Lure `#FFC043`, Coral `#F0705F`.
- Lure means only "sound is happening here" or "this is where you are", plus the one main button per dialog.
- Rows 26 px high; text 13 px (rows), 13.5 px (chat), 12 px (secondary); weights 400 and 600.
- Sentence case everywhere. No capitalised labels, no emoji, icons are SVG files tinted with theme colours.
- Status line wording matches the settings: "Sends when I speak", "Sends while I hold <key>", "Always sends".
- Window about 400 x 740 by default, minimum 340 x 520, size remembered. Settings window about 700 x 470.
- No comments in code (project convention). Existing tests keep passing.
- Bookmarks never store passwords.

## Review Focus

Conditions the spec implies that are most likely to bite, each pinned to a task below:

1. Switching the viewed server while talking: the server you leave must receive an end-of-talk packet, and voice must reach only the new one. (Task 4 unit test on routing decisions; Task 6 live check.)
2. The viewed server disconnects while others stay connected: the view moves to another session and the microphone follows. (Task 4.)
3. Two servers hand out the same client number: both people are heard, neither cuts the other off, and forgetting one connection does not silence the other. (Task 2.)
4. Names that only look like spacers (`[cspacer` with no bracket, `[xspacer]Text`, a `[cspacer]` channel that is not top-level): shown as ordinary channels and still joinable. (Task 1.)
5. A damaged or hand-edited bookmarks file: bad entries are skipped, good ones load, the app starts. (Task 3.)

## File Structure

| File | Responsibility |
|---|---|
| `ps-client/src/spacer.rs` (new) | Decide whether a channel is a spacer and how to draw it |
| `ps-voice/src/playback.rs`, `lib.rs`, `state.rs` (modify) | Talkers keyed by connection and client |
| `ps-app/src/bookmarks.rs` (new) | Bookmark list, initials, load and save |
| `ps-app/src/session.rs` (new) | One connection: client handle, events, view, chat history, tree rows |
| `ps-app/src/app.rs` (new) | All sessions, the viewed one, microphone routing, UI updates |
| `ps-app/src/main.rs` (rewrite) | Start-up, wiring callbacks, the UI timer |
| `ps-app/src/settings.rs` (modify) | Add window size; keep audio and identity settings |
| `ps-app/ui/theme.slint` (new) | Palette and sizes as one global |
| `ps-app/ui/widgets.slint` (new) | Tile, icon button, meter, tree rows, chat row, segmented switch |
| `ps-app/ui/main.slint` (rewrite) | The compact window, servers menu, connect dialog, password prompt |
| `ps-app/ui/settings.slint` (new) | The settings window and its six tabs |
| `ps-app/ui/icons/*.svg` (new) | microphone, microphone-off, headphones, headphones-off, cog, lock, speaker, note, plus, bookmark, close, chevron, signal |

---

### Task 1: Spacer channels in `ps-client`

**Files:** Create `ps-client/src/spacer.rs`; modify `ps-client/src/lib.rs` (export).

**Interfaces:** Produces
`pub enum SpacerAlign { Left, Center, Right, Repeat }`,
`pub enum SpacerLine { Solid, Dashed, Dotted }`,
`pub enum Spacer { Text { align: SpacerAlign, text: String }, Line(SpacerLine), Gap }`,
`pub fn parse_spacer(name: &str, parent: u64) -> Option<Spacer>`.

- [x] Write the failing tests. Cases, `(name, parent) -> result`:
  `[cspacer]Games` -> Text Center "Games"; `[lspacer]Chill` and `[spacer]Chill` -> Text Left;
  `[rspacer]est. 2019` -> Text Right; `[*spacer]-=` -> Text Repeat "-="; `[cspacer12]Games` and
  `[CSPACER]Games` -> Text Center; `[spacer0]---`, `-.-`, `-..` -> Line Dashed; `...` -> Line Dotted;
  `___` -> Line Solid; `[spacer3]` and `[cspacer]   ` -> Gap; and None for `[cspacer`, `[xspacer]A`,
  `cspacer]A`, `Lobby`, `[cspacer]Games` with parent 5, `[c spacer]A`.
- [x] Run `cargo test -p ps-client spacer`; expect compile failure.
- [x] Implement `parse_spacer`: parent must be 0; name must start with `[`; the tag up to the first `]`, lower-cased, must be `l`, `c`, `r`, `*` or nothing followed by `spacer` and then anything; the rest is the text.
- [x] Run the tests; expect pass.

### Task 2: Mixer keyed by connection and client in `ps-voice`

**Files:** Modify `ps-voice/src/playback.rs`, `ps-voice/src/lib.rs`, `ps-voice/src/state.rs`, `ps-voice/examples/voicetest.rs`, `ps-voice/examples/devicetest.rs`.

**Interfaces:** Produces
`Playback::push(&mut self, session: u16, client_id: u16, voice_id: u16, codec: u8, data: &[u8])`,
`Playback::remove(session, client_id)`, `Playback::clear_session(session)`, `Playback::talker(session, client_id)`,
`AudioEngine::push_voice(session, client_id, voice_id, codec, data)`, `AudioEngine::remove_talker(session, client_id)`,
`AudioEngine::clear_session(session)`, `pub const LOOPBACK_SESSION: u16 = 0xffff`.

- [x] Write the failing test `two_connections_can_share_a_client_number`: push 30 packets of a 440 Hz stream as (session 1, client 7) and 30 of an 880 Hz stream as (session 2, client 7); mix 10 blocks; assert two active talkers and that both `talker(1, 7)` and `talker(2, 7)` decoded 10 packets; `clear_session(1)`; assert one active talker remains and `talker(2, 7)` still exists.
- [x] Run `cargo test -p ps-voice two_connections`; expect compile failure.
- [x] Change the talker map key to `(u32::from(session) << 16) | u32::from(client_id)`; add `clear_session`; thread the session through `AudioEngine` and the loopback path; update the existing mixer test and both examples to pass session 0.
- [x] Run `cargo test -p ps-voice`; expect all pass.

### Task 3: Bookmarks in `ps-app`

**Files:** Create `ps-app/src/bookmarks.rs`.

**Interfaces:** Produces
`pub struct Bookmark { pub name: String, pub address: String, pub nickname: String, pub identity_uid: String }`,
`pub struct Bookmarks { pub items: Vec<Bookmark> }` with `parse(&str) -> Self`, `serialize(&self) -> String`, `load() -> Self`, `save(&self) -> io::Result<()>`, `upsert(&mut self, Bookmark) -> usize` (same address, ignoring case, replaces), `remove(&mut self, usize)`,
`pub fn initials(name: &str) -> String`.
File: `%APPDATA%\PhishSpeak\bookmarks.ini`, one `[bookmark]` section per entry with `name`, `address`, `nickname`, `identity`.

- [x] Write the failing tests: round trip of three bookmarks including a name with `=` and non-ASCII letters and an identity UID ending in `=`; a damaged file (stray lines, a section with no address, an empty section) keeps only the complete entries; `upsert` replaces an entry with the same address in different case and appends a new one; `initials`: "Reef Runners" -> "RR", "night shift raids" -> "NS", "Home" -> "HO", "x" -> "X", "" -> "?", "  the   reef " -> "TR".
- [x] Run `cargo test -p ps-app bookmarks`; expect compile failure.
- [x] Implement. Values are written after the first `=` on the line and read back with `split_once('=')`; an entry needs a non-empty address to count; a missing name falls back to the address.
- [x] Run the tests; expect pass.

### Task 4: Sessions and routing in `ps-app`

**Files:** Create `ps-app/src/session.rs`, `ps-app/src/app.rs`; modify `ps-app/src/settings.rs`.

**Interfaces:** Consumes Tasks 1 to 3. Produces
`pub struct RowData { kind: RowKind, id: u64, depth: u32, text: String, align: SpacerAlign, line: SpacerLine, icon: ChannelIcon, count: usize, current: bool, talking: bool, me: bool, mic_muted: bool, sound_muted: bool, away: bool, tag: String }`,
`pub enum RowKind { SpacerText, SpacerLine, Gap, Channel, Person }`, `pub enum ChannelIcon { Speaker, Lock, Music }`,
`pub fn build_rows(view: &ServerView, own_talking: bool) -> Vec<RowData>`,
`pub struct ChatLine { time: String, kind: ChatKind, name: String, text: String }`, `pub enum ChatKind { System, Message, Mine, Error }`,
`pub struct Session { id: u16, name: String, address: String, client: ClientHandle, events: Receiver<Event>, view: Option<ServerView>, chat: VecDeque<ChatLine>, phase: Phase, ping: String, talkers: usize }`,
`pub enum Phase { Connecting, Connected, Closed(String) }`,
`pub fn next_view(sessions: &[(u16, bool)], viewed: Option<u16>, closed: u16) -> Option<u16>` (which session to show after one closes: the next connected one, else any remaining, else none),
`pub enum MicMove { None, Switch { end_talk_on: Option<u16>, send_to: Option<u16> } }`,
`pub fn mic_move(old: Option<u16>, new: Option<u16>, transmitting: bool) -> MicMove`.

- [x] Write the failing tests. `build_rows`: a view with a centred spacer, a line spacer, a locked channel, a music channel (codec 5) with a nested channel, three people (me, one talking, one muted and away) produces the expected kinds, depths, icons, counts and flags in tree order, and spacers have count 0. `next_view`: closing the viewed session picks the next connected one; closing a background session keeps the view; closing the last returns None. `mic_move`: same session -> None; different session while transmitting -> end talk on the old, send to the new; while silent -> no end-talk; new None -> send to nobody. Settings: window width and height round-trip and clamp to the minimum.
- [x] Run `cargo test -p ps-app`; expect compile failure.
- [x] Implement `session.rs` (row building, chat history capped at 400 lines, event handling moved over from the current `main.rs`) and the pure functions. Implement `app.rs`: connect from a bookmark or from dialog fields, switch view (apply `mic_move`: send an empty voice packet to the session left behind, point the engine's frame sink at the new one, set the codec from its channel), disconnect, mute applied to every session, the security-level retry per session, and one voice sink per session that tags packets with the session id. When a server refuses the password or cannot be reached, the connect dialog reopens with the fields still filled and the reason under the field it concerns; the password is used for that attempt only and never written to disk.
- [x] Run `cargo test -p ps-app`; expect pass.

### Task 5: The Slint interface

**Files:** Create `ps-app/ui/theme.slint`, `widgets.slint`, `settings.slint`, `icons/*.svg`; rewrite `ps-app/ui/main.slint`; modify `ps-app/build.rs` only if the include paths need it.

**Interfaces:** Consumes the Task 4 types, converted to Slint structs `TreeRow`, `ServerTile`, `ChatRow`, `BookmarkRow`, `IdentityRow`. Produces two components, `PhishSpeakApp` (main window) and `SettingsWindow`, with these callbacks to Rust: `open-menu`, `view-server(id)`, `connect-bookmark(index)`, `connect-new(address, nickname, password, identity-index, save)`, `disconnect-viewed`, `row-activated(row)`, `join-with-password(channel-id, password)`, `send-chat(text, to-server)`, `toggle-mic`, `toggle-sound`, `open-settings(tab)` (the cog opens the Microphone tab, "Edit bookmarks" opens the Bookmarks tab); and from the settings window: `audio-changed`, `input-device-selected`, `output-device-selected`, `new-identity`, `import-identity(path)`, `bookmark-saved(index, name, address, nickname, identity-index)`, `bookmark-removed(index)`, `ptt-key-selected`.

- [x] Write `theme.slint` with the eight colours and the sizes from Global Constraints.
- [x] Write the icons as 24 x 24 stroke-only SVG files and check each loads with `colorize`.
- [x] Write `widgets.slint`: `Tile` (initials, connected ring, viewed fill, talking dot), `IconButton` (32 px, hover, on and muted states), `LevelMeter` with marker, `TreeRowView` (one component switching on row kind; the talking dot gets a halo that grows in over 250 ms), `ChatRowView`, `Segmented`.
- [x] Rewrite `main.slint`: header, tree `ListView`, chat drawer (closed and open), dock, empty state, and three overlays: servers menu, connect dialog (error text under the address field), channel password prompt.
- [x] Write `settings.slint`: a themed tab strip and six pages (Microphone, Sound, Identities, Bookmarks, Shortcuts, About) using fluent `ComboBox`, `Slider`, `LineEdit` and `CheckBox` for input.
- [x] `cargo build -p ps-app`; expect a clean build with no warnings.

### Task 6: Wiring, live checks and documents

**Files:** Rewrite `ps-app/src/main.rs`; modify `PLAN.md`, `README.md`.

- [x] Wire every callback from Task 5 to `app.rs`; one 33 ms timer pumps all sessions and refreshes only what changed; closing the main window disconnects every session and saves settings and bookmarks.
- [x] `cargo test --workspace` and `cargo build --workspace --all-targets`; expect all tests pass and no warnings.
- [x] Start the local TeamSpeak server. Create spacer channels through ServerQuery (`[cspacer]Games`, `[*spacer1]---`, `[lspacer]Chill`, `[rspacer]est. 2019`, `[spacer2]...`) and a channel named `[cspacer` to confirm it stays joinable.
- [x] Take software-rendered screenshots of: empty state, in a channel, servers menu, chat open, connect dialog, password prompt, each settings tab. Compare with `compact-detail-v2.html`; fix differences.
- [x] Two sessions at once with two identities in different channels. With a headless listener in each channel, confirm voice arrives only in the viewed session's channel, that switching view moves it, and that the session left behind gets its end-of-talk packet. Confirm a headless sender in each channel is heard from both.
- [x] Bookmark a server, close the app, start it again, connect from the bookmark with one click.
- [x] Check the spec's quality bar: contrast of Drift on Reef and Foam on Current at least 4.5:1, visible focus ring, hover on every clickable element, nothing clickable under 28 px.
- [x] Update `PLAN.md` (status, crates table, Slint notes learned) and the README's run instructions. Run `build.bat` once more.
