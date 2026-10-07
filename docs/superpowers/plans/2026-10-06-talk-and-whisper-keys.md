# Talk Keys and Whisper Keys Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let any key, mouse button or key combination be a talk key, and add whisper keys that send your voice to chosen channels and people, or to a group of people in the channels above, below or around yours, the way TeamSpeak 3 whisper lists do.

**Architecture:** A small thread reads the keyboard and mouse state every 5 ms and tells the audio engine two things: is a talk key held, and which whisper key (if any) is held. The audio engine tags every outgoing frame with a lane: lane 0 is ordinary talk, lanes 1 and up are whisper keys. The app turns a lane into a destination: `send_voice` for lane 0, a whisper packet with that key's targets for the others, and nothing at all when a whisper key has no usable target. `ps-protocol` and `ps-client` gain the two whisper packet forms.

**Tech Stack:** Rust stable, Slint 1.18, existing workspace crates, two more Win32 calls through the FFI block `platform.rs` already has. No new dependencies.

**Spec:** none. The next two sections, the scope and the decisions, served as one.

**Status:** built and checked on 2026-10-06. Every step is done except Task 9 Step 9, the check with the official client, which only you can do. What was decided on the way, and what you added while it ran, is in `2026-10-06-talk-and-whisper-keys.ledger.md`.

**How to read the tasks:** tests and the functions where a mistake would send voice to the wrong people are given in full. Steps that connect existing code are described by what they must do and are held to the tests and live checks listed with them. Every Rust block below was compiled on 2026-10-06 in a throwaway copy of the workspace with the changes it describes, and every test in this plan passed there (protocol 34, client 12, audio 34, and the app-side tests); the `whispertest` example printed twelve `PASS` lines against the test server. Nothing was changed in the repository. Part A (Tasks 1 to 3) is usable on its own; Part B (Tasks 4 to 9) needs Part A. This plan does not depend on the custom-icons plan; both touch `session.rs`, `app.rs` and `widgets.slint`, so whichever runs second merges by hand.

## What this builds

**Part A, talk keys.** In Settings, Shortcuts, a talk key is set by clicking it and pressing what you want: one key, a mouse button (middle, 4 or 5), or a combination of up to four keys. You can have several talk keys. A slider keeps the microphone open for a moment after you let go, so the end of a word is not cut off. Your current key carries over.

**Part B, whisper keys.** A whisper key is a second kind of talk key. While you hold it, your voice goes to that key's targets and not to your channel. Up to twelve of them. Each one aims at either:

- **channels and people** you tick in the channel tree of the server you are looking at, or
- **a group**: everyone, channel commanders, one server group or one channel group, in one of seven places relative to you.

```
Shortcuts
  Talk keys          [ Left Ctrl  x ]  [ Mouse 4  x ]  [ Add a key ]
  After I let go     keep sending for  [----o-------]  0.2 s

  Whisper keys
    [ Num 5 ]   Channel commanders, my whole branch          Edit   Remove
    [ Num 4 ]   Lobby and 2 more  ·  Reef Runners            Edit   Remove
    [ Add a whisper key ]

  Reply key          [ Num 0  x ]     talks to whoever whispered to you last
  [x] Let others whisper to me
```

The seven places, with what the test server did for each (your channel is the one you are in when you press the key):

| Shown as | Reaches |
|---|---|
| Everywhere | every channel on the server |
| My channel | your channel only |
| The channel above mine | the parent channel only |
| Every channel above mine | the parent, its parent, and so on up; not your own channel |
| My channel and all below it | your channel and every channel under it, at any depth |
| My whole branch, from the top | the top-level channel you are under and everything inside it, including branches beside yours |
| The channels right below mine | the channels directly under yours; not the ones under those |

Also in Part B, because whisper keys are of little use without them:

- A **reply key** that whispers to whoever whispered to you last.
- People who are whispering to you are marked in the tree, and a switch stops whispers from being played at all.
- **Channel commander**: a menu entry to become one (servers must allow it), a mark on people who are one, and "channel commanders" as a whisper group. This is the most common whisper setup on TeamSpeak servers.
- The dock says who a whisper is going to, and says so when nobody could hear it.

## Decisions

**Confirmed by you on 2026-10-06:**

1. **A whisper key opens the microphone by itself**, whatever "Send my voice" is set to. That is how TeamSpeak 3 behaves: the key is an extra push-to-talk. With "When I speak" selected, a held whisper key still sends everything, including silence.
2. **Whisper keys act on the server you are looking at**, like the microphone does. A key that names channels, people or a group of another server does nothing there and the dock says which server it belongs to.
3. **A whisper key never falls back to your channel.** If its targets are gone, offline or on another server, nothing is sent.
4. **The release delay is a setting and starts at 0.** It is the slider "After I let go" on the Shortcuts tab, from 0 to 1 second (TeamSpeak allows 3). At 0 nothing changes from today. It applies to talk keys and whisper keys alike.
5. **Left and right mouse buttons cannot be talk keys.** They are needed to click. Middle, 4 and 5 can.

**Accepted when you said "go" on 2026-10-06, with game controllers left out for now:**

6. **Keys are read by asking Windows 200 times a second whether each of your keys is down** (polling). That is what the talk key does today. The other way is a keyboard hook, which passes every key press on the PC through PhishSpeak as it happens. For keyboard and mouse both work while a game has the focus. Polling is simpler, cannot leave a key stuck, and is not the kind of system-wide hook that anti-cheat and antivirus tools sometimes flag; a hook would react the instant a key moves, not up to 5 ms later, which nobody can hear. You answered "not sure"; the recommendation is polling. Two limits apply whichever way is chosen, so they are not a reason to prefer either: a game that runs as administrator may hide the keys from PhishSpeak unless PhishSpeak also runs as administrator (not tested), and buttons on game controllers, joysticks and pedals are not keyboard keys, so neither way sees them. Controller buttons are not in this plan; they can be added later as their own task without redoing this design.
7. **Whispers are always sent in the speech sound format (Opus Voice), also when you sit in a channel set to music quality.** A whisper packet has to carry the list of who it is for as well as the sound, and a packet holds 500 bytes. A music-quality piece of sound can take most of that, so with a long list the packet would be too big and nobody would hear the whisper. What you would notice: a whisper sent from a music channel sounds like ordinary speech, not hi-fi stereo. Ordinary talk in that channel is unaffected.
8. **Limits:** twelve whisper keys, thirty channels and sixty people per key, four keys per combination.
9. **"Let others whisper to me" has two settings**, on and off. TeamSpeak also has a per-contact choice; PhishSpeak has no contact list.
10. **Not in this plan:** other hotkey actions (mute, switch channel, and so on), toggle-to-talk, per-server key profiles, a whisper history window, a sound when someone whispers to you, and whisper keys that only send while you speak.

## Facts this plan rests on

Checked on 2026-10-06. "Live" means against the local TeamSpeak 3.13.8 test server with eight test clients, using a throwaway copy of the client code outside the repository.

**The packets** (tsdeclarations `ts3protocol.md` 1.8.2, tsclientlib `OutAudio`, TSLib `SendAudioWhisper` and `SendAudioGroupWhisper` all agree, and live confirmed both):

- To a list, packet type VoiceWhisper, `Newprotocol` flag clear: `[voice id u16][codec u8][N u8][M u8][N channel ids, u64 each][M client ids, u16 each][voice data]`, big-endian.
- To a group, packet type VoiceWhisper, `Newprotocol` flag set: `[voice id u16][codec u8][who u8][where u8][id u64][voice data]`. Who: 0 server group, 1 channel group, 2 channel commanders, 3 everyone. Where: 0 to 6 in the order of the table above. The id is the group id for kinds 0 and 1 and is ignored otherwise (live: passing a channel id changed nothing).
- Received: `[voice id u16][from client id u16][codec u8][voice data]`, the same as ordinary voice, told apart by packet type. `conn.rs` already passes this on as `VoicePacket::whisper`.
- An empty voice part ends the whisper, as it does for talk. Live: the listener received it.
- `Conn::send_packet` already encrypts VoiceWhisper by the same rule as Voice and stamps the voice id. Live: whispers arrived with voice encryption forced on, forced off and per channel.

**Who hears what** (live; tree: Deep Rock contains Radio and Drift, Radio contains Booth; Lobby and Tide Pool are separate top-level channels):

| Where | Sender in Radio | Sender in Booth | Sender in Deep Rock |
|---|---|---|---|
| all channels | everyone on the server | | |
| current channel | Radio | Booth | Deep Rock |
| parent channel | Deep Rock | Radio | nobody |
| all parent channels | Deep Rock | Radio, Deep Rock | nobody |
| channel family | Radio, Booth | Booth | Deep Rock, Radio, Booth, Drift |
| complete channel family | Deep Rock, Radio, Booth, Drift | the same four | the same four |
| subchannels | Booth | nobody | Radio, Drift (not Booth) |

- A list reaches exactly the people in the listed channels plus the listed people. Listing your own channel reaches your channel-mates as a whisper. Ids that do not exist are skipped; thirty channel ids in one packet worked.
- The group filter and the place combine: "server group Guest, parent channel" reached only the guest in the parent channel. "Channel commanders, all channels" reached only the one client with the commander flag.
- The codec byte is passed through untouched: a codec 5 whisper reached a listener in a codec 4 channel as codec 5.

**What the server tells the sender** (live):

- When a whisper reaches nobody, the server answers `error id=1804 (0x070c) msg=no whisper targets found`, once per burst, not per packet. This happened for an empty list, for a group or place that matched nobody, for an unknown who or where value, and when the listeners required more whisper power than the sender had. The cases cannot be told apart.
- On a fresh server guests can whisper and be whispered to. With `i_client_needed_whisper_power=50` on the Guest group, nothing arrived and the sender got the error above; ordinary talk was unaffected.
- `clientupdate client_is_channel_commander=1` is refused with `error id=2568 (0x0a08) insufficient client permissions` unless the server grants `b_client_use_channel_commander`. Guests do not have it by default.

**Keys** (measured on this PC):

- `GetAsyncKeyState` sees keys while another program has the focus; it is what the current talk key already uses. Reading all 254 key states takes 0.18 ms.
- `std::thread::sleep(5 ms)` took 5.22 ms on average and 5.83 ms at worst over 200 rounds.
- A key press injected with `keybd_event` for F23 or F24 (keys no keyboard has) was seen by `GetAsyncKeyState`, alone and as a pair. That gives the live checks a way to press a global key without touching a real one.
- The system's own key names are not usable as they are: `GetKeyNameTextW` called "Left Ctrl" just "Ctrl", gave nothing for F13 to F24 and the mouse buttons, and called Insert "Num 0" and Pause "Right Ctrl".

**How TeamSpeak 3 behaves** (read, not run):

- The place names and their meanings are in TeamSpeak's plugin SDK header, `GroupWhisperTargetMode` in `public_definitions.h`. Its wording for the last-but-one is "the current channel, all its parent and sub channels"; the live result above shows it also includes the branches beside yours.
- A whisper list is a hotkey plus "Whisper to: Clients and Channels" or "Groups", with "Group Whisper Type" and "Group Whisper Target"; the hotkey "functions as an additional push-to-talk button" (Badger's Guide to TeamSpeak). A forum request to add voice activation to whisper keys confirms they send without it.
- TeamSpeak has a "Push-To-Reply to a Whisper" hotkey, an "Allow whispers from" option, and "Delay releasing Push-To-Talk" in steps of 0.1 s up to 3 s. These come from search summaries of forum and guide pages; I did not open a TeamSpeak client to confirm them.

Not checked: whispering to or from the official TeamSpeak client (everything live was PhishSpeak to PhishSpeak), and keys while an administrator-level game has the focus.

Pages read for the TeamSpeak behaviour: `include/teamspeak/public_definitions.h` and `include/ts3_functions.h` in github.com/teamspeak/ts3client-pluginsdk; badgerteamspeak.blogspot.com/2014/03/teamspeak-3-whisper-lists-and-channel.html; forum.teamspeak.com threads 59202 (voice activation for whispers), 53123 (whisper to channel family) and 120684 (push-to-talk delay).

## Global Constraints

- No new crates.
- A whisper key must never cause ordinary channel voice to be sent.
- Every audience gets an end-of-talk packet when the stream to it stops, including when the key changes mid-sentence.
- A whisper packet must never exceed the 487-byte payload limit: the frame is encoded into whatever room the targets leave, and lists are limited to 30 channels and 60 people (a 365-byte header, leaving 122 bytes).
- Keys are polled; no keyboard or mouse hook is installed. While a key is being chosen in settings, nothing is sent.
- Old settings keep working: `ptt_key=<number>` is read once and rewritten as `talk_key=`.
- Whisper keys are stored in `%APPDATA%\PhishSpeak\whisper.ini`. People are stored by their TeamSpeak UID and last known name, channels by id and last known name, groups by id, name and the server's UID.
- Wording: "Whisper keys", "Talk keys", sentence case, the seven places exactly as in the table under "What this builds".
- Amber keeps its three meanings; a key being chosen shows the amber focus ring because it has the focus.
- Controls are at least 28 px high, have a hover state and a visible focus ring.
- No comments in code. No real identities, UIDs or machine paths in tests, documents or scripts. Live checks use generated identities and a throwaway profile with output volume 0.
- Existing tests keep passing (120 today).

## Review Focus

Conditions the scope implies that are most likely to bite, each pinned to a task:

1. Holding a whisper key whose targets are gone, offline, or belong to another server, or pressing one a moment after switching servers: nothing may go to the channel, and the dock must say why nothing is being sent. (Task 6 unit tests `a_lane_never_falls_back_to_talk` and `targets_follow_the_server`; Task 9 live check with a listener in the channel.)
2. Changing audience mid-sentence: pressing a whisper key while talking, letting go of it first, or sliding from one whisper key to another. Each audience must get its end-of-talk packet and no frame meant for another. (Task 5 unit test `changing_lane_mid_sentence_ends_the_old_one_first`; Task 9 live check with listeners.)
3. A key that seems stuck or dead: settings closed while a key is being chosen, the same key chosen for two things, a combination that contains another (Ctrl and Ctrl+1). Nothing may stay open, and the more specific key wins. (Task 1 and Task 2 unit tests; Task 3 live check.)
4. A settings file from today's build: the talk key must still work after the update without the user doing anything. (Task 3 unit test `old_talk_key_setting_is_carried_over`.)
5. Many targets, or whispering from a music channel: the packet must still fit and still be delivered. (Task 4 unit test on lengths, Task 5 unit test `whisper_frames_fit_their_lane_and_use_the_voice_codec`.)

## File Structure

| File | Responsibility |
|---|---|
| `ps-app/src/hotkeys.rs` (new) | Key combinations, key names, which keys count as held, release delay, choosing a key |
| `ps-app/src/keywatch.rs` (new) | The thread that reads the keys and tells the audio engine |
| `ps-app/src/platform.rs` (modify) | `key_char`; the fixed key list moves out |
| `ps-app/src/settings.rs` (modify) | Talk keys, release delay, reply key, the whisper switch; reading the old setting |
| `ps-app/src/whisper.rs` (new) | Whisper keys: what they aim at, their file, turning one into a target on the viewed server, describing one |
| `ps-app/src/session.rs`, `app.rs` (modify) | Lanes to destinations, dock wording, people whispering to you, reply target, commander |
| `ps-app/ui/settings.slint`, `widgets.slint`, `main.slint` (modify) | Shortcuts tab, whisper key editor, marks in the tree, menu entry |
| `ps-voice/src/state.rs`, `capture.rs`, `lib.rs` (modify) | Lanes: which key is held, frame room per lane, end markers on change |
| `ps-protocol/src/voice.rs` (modify) | The two whisper payloads |
| `ps-client/src/lib.rs`, `conn.rs`, `book.rs` (modify) | `WhisperTarget`, `send_whisper`, who is whispering, commander |
| `ps-client/examples/whispertest.rs` (new) | Re-runs the who-hears-what table against a server |
| `ps-voice/examples/channeltest.rs` (modify) | Says whether what it heard was talk or a whisper |
| `tools/hold_keys.py`, `tools/seed_whisper_tree.py` (new) | Press F13 to F24 for a live check; build the test channel tree |

---

## Part A: talk keys

### Task 1: Combinations, names and what counts as held

**Files:** Create `ps-app/src/hotkeys.rs`; modify `ps-app/src/main.rs` (`mod hotkeys;`).

**Interfaces:** Produces
`pub const MAX_CHORD_KEYS: usize = 4;`, `pub const MAX_WHISPER_KEYS: usize = 12;`, `pub const REPLY_LANE: u8 = 13;`,
`pub fn usable(vk: u16) -> bool`,
`pub struct Chord` with `new(&[u16]) -> Self`, `keys(&self) -> &[u16]`, `is_empty`, `parse(&str) -> Self`, `to_text(&self) -> String`, `is_down(&self, down: &dyn Fn(u16) -> bool) -> bool`,
`pub fn key_name(vk: u16, character: &dyn Fn(u16) -> Option<char>) -> String`, `pub fn chord_name(chord: &Chord, character: &dyn Fn(u16) -> Option<char>) -> String`,
`pub struct Bindings { pub talk: Vec<Chord>, pub whisper: Vec<Chord>, pub reply: Chord }`,
`pub struct Held { pub talk: bool, pub lane: u8 }` (lane 0: no whisper key; 1 to 12: that whisper key; 13: the reply key),
`pub fn evaluate(bindings: &Bindings, down: &dyn Fn(u16) -> bool) -> Held`,
`pub struct Latch` with `update(&mut self, now: Instant, raw: Held, delay: Duration) -> Held`,
`pub enum CaptureStep { Waiting, Done(Chord), Cancelled }`, `pub struct Capture` with `new()` and `feed(&mut self, down: &[u16]) -> CaptureStep`.
Keys are Windows virtual-key codes.

- [x] **Step 1: Write the failing tests** in `hotkeys.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn down_set(keys: &[u16]) -> impl Fn(u16) -> bool + '_ {
        move |vk| keys.contains(&vk)
    }

    #[test]
    fn chords_are_normalised_and_round_trip() {
        let chord = Chord::new(&[0x41, 0xA2, 0x41, 0x11, 0x01]);
        assert_eq!(chord.keys(), &[0x41, 0xA2]);
        assert_eq!(chord.to_text(), "65+162");
        assert_eq!(Chord::parse("65+162"), chord);
        assert_eq!(Chord::parse(" 162 + 65 "), chord);
        assert!(Chord::parse("").is_empty());
        assert!(Chord::parse("abc+0+999+1+2").is_empty());
        assert_eq!(Chord::parse("135+134+133+132+131").keys().len(), MAX_CHORD_KEYS);
        assert!(!Chord::default().is_down(&|_| true));
        assert!(chord.is_down(&down_set(&[0x41, 0xA2, 0x20])));
        assert!(!chord.is_down(&down_set(&[0xA2])));
    }

    #[test]
    fn keys_have_readable_names() {
        let letters = |vk: u16| match vk {
            0x30..=0x5A => char::from_u32(u32::from(vk)),
            0xC0 => Some('`'),
            _ => None,
        };
        assert_eq!(key_name(0xA2, &letters), "Left Ctrl");
        assert_eq!(key_name(0xA5, &letters), "Right Alt");
        assert_eq!(key_name(0x04, &letters), "Mouse 3");
        assert_eq!(key_name(0x05, &letters), "Mouse 4");
        assert_eq!(key_name(0x2D, &letters), "Insert");
        assert_eq!(key_name(0x13, &letters), "Pause");
        assert_eq!(key_name(0x65, &letters), "Num 5");
        assert_eq!(key_name(0x7C, &letters), "F13");
        assert_eq!(key_name(0x87, &letters), "F24");
        assert_eq!(key_name(0x41, &letters), "A");
        assert_eq!(key_name(0xC0, &letters), "`");
        assert_eq!(key_name(0xE8, &letters), "Key 232");
        assert_eq!(chord_name(&Chord::new(&[0x41, 0xA0, 0xA2]), &letters), "Left Ctrl + Left Shift + A");
        assert_eq!(chord_name(&Chord::default(), &letters), "");
    }

    #[test]
    fn the_most_specific_whisper_key_wins() {
        let bindings = Bindings {
            talk: vec![Chord::new(&[0xA2]), Chord::new(&[0x05])],
            whisper: vec![Chord::new(&[0x65]), Chord::new(&[0xA2, 0x31]), Chord::new(&[0x31])],
            reply: Chord::new(&[0x60]),
        };
        assert_eq!(evaluate(&bindings, &down_set(&[])), Held { talk: false, lane: 0 });
        assert_eq!(evaluate(&bindings, &down_set(&[0xA2])), Held { talk: true, lane: 0 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x05])), Held { talk: true, lane: 0 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x65])), Held { talk: false, lane: 1 });
        assert_eq!(evaluate(&bindings, &down_set(&[0xA2, 0x31])), Held { talk: true, lane: 2 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x31])), Held { talk: false, lane: 3 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x65, 0x31])), Held { talk: false, lane: 1 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x60])), Held { talk: false, lane: REPLY_LANE });
        assert_eq!(evaluate(&Bindings::default(), &down_set(&[0xA2, 0x65])), Held::default());
    }

    #[test]
    fn release_is_delayed_but_presses_are_not() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let delay = Duration::from_millis(200);
        let talk = Held { talk: true, lane: 0 };
        let none = Held::default();
        let lane = |n: u8| Held { talk: false, lane: n };
        let mut latch = Latch::default();
        assert_eq!(latch.update(at(0), talk, delay), talk);
        assert_eq!(latch.update(at(100), none, delay), talk);
        assert_eq!(latch.update(at(299), none, delay), talk);
        assert_eq!(latch.update(at(300), none, delay), none);
        assert_eq!(latch.update(at(400), lane(2), delay), lane(2));
        assert_eq!(latch.update(at(410), lane(3), delay), lane(3));
        assert_eq!(latch.update(at(420), none, delay), lane(3));
        assert_eq!(latch.update(at(500), lane(3), delay), lane(3));
        assert_eq!(latch.update(at(600), none, delay), lane(3));
        assert_eq!(latch.update(at(800), none, delay), none);
        let mut instant = Latch::default();
        assert_eq!(instant.update(at(0), talk, Duration::ZERO), talk);
        assert_eq!(instant.update(at(1), none, Duration::ZERO), none);
    }

    #[test]
    fn capture_waits_for_a_clean_start_and_a_full_release() {
        let mut capture = Capture::new();
        assert_eq!(capture.feed(&[0xA2]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[0xA2]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[0xA2, 0x41]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[0x41]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[]), CaptureStep::Done(Chord::new(&[0xA2, 0x41])));

        let mut cancelled = Capture::new();
        cancelled.feed(&[]);
        assert_eq!(cancelled.feed(&[0x1B]), CaptureStep::Cancelled);

        let mut with_escape = Capture::new();
        with_escape.feed(&[]);
        with_escape.feed(&[0xA2]);
        with_escape.feed(&[0xA2, 0x1B]);
        assert_eq!(with_escape.feed(&[]), CaptureStep::Done(Chord::new(&[0xA2, 0x1B])));

        let mut never_armed = Capture::new();
        assert_eq!(never_armed.feed(&[0x41]), CaptureStep::Waiting);
        assert_eq!(never_armed.feed(&[0x41]), CaptureStep::Waiting);
    }
}
```

- [x] **Step 2:** Run `cargo test -p ps-app hotkeys`; expect a compile failure.
- [x] **Step 3: Implement.**

```rust
use std::time::{Duration, Instant};

pub const MAX_CHORD_KEYS: usize = 4;
pub const MAX_WHISPER_KEYS: usize = 12;
pub const REPLY_LANE: u8 = 13;
const ESCAPE: u16 = 0x1B;

pub fn usable(vk: u16) -> bool {
    (3..=254).contains(&vk) && !matches!(vk, 0x10 | 0x11 | 0x12)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Chord(Vec<u16>);

impl Chord {
    pub fn new(keys: &[u16]) -> Self {
        let mut list: Vec<u16> = keys.iter().copied().filter(|vk| usable(*vk)).collect();
        list.sort_unstable();
        list.dedup();
        list.truncate(MAX_CHORD_KEYS);
        Self(list)
    }

    pub fn keys(&self) -> &[u16] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn parse(text: &str) -> Self {
        let keys: Vec<u16> = text.split('+').filter_map(|part| part.trim().parse::<u16>().ok()).collect();
        Self::new(&keys)
    }

    pub fn to_text(&self) -> String {
        self.0.iter().map(|vk| vk.to_string()).collect::<Vec<String>>().join("+")
    }

    pub fn is_down(&self, down: &dyn Fn(u16) -> bool) -> bool {
        !self.0.is_empty() && self.0.iter().all(|vk| down(*vk))
    }
}

pub fn key_name(vk: u16, character: &dyn Fn(u16) -> Option<char>) -> String {
    let fixed = match vk {
        0x04 => "Mouse 3",
        0x05 => "Mouse 4",
        0x06 => "Mouse 5",
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x13 => "Pause",
        0x14 => "Caps Lock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2C => "Print Screen",
        0x2D => "Insert",
        0x2E => "Delete",
        0x5B => "Left Win",
        0x5C => "Right Win",
        0x5D => "Menu",
        0x6A => "Num *",
        0x6B => "Num +",
        0x6D => "Num -",
        0x6E => "Num .",
        0x6F => "Num /",
        0x90 => "Num Lock",
        0x91 => "Scroll Lock",
        0xA0 => "Left Shift",
        0xA1 => "Right Shift",
        0xA2 => "Left Ctrl",
        0xA3 => "Right Ctrl",
        0xA4 => "Left Alt",
        0xA5 => "Right Alt",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    if (0x60..=0x69).contains(&vk) {
        return format!("Num {}", vk - 0x60);
    }
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x6F);
    }
    match character(vk) {
        Some(c) if !c.is_control() && !c.is_whitespace() => c.to_uppercase().collect(),
        _ => format!("Key {vk}"),
    }
}

fn rank(vk: u16) -> u8 {
    match vk {
        0xA2 | 0xA3 => 0,
        0xA0 | 0xA1 => 1,
        0xA4 | 0xA5 => 2,
        0x5B | 0x5C => 3,
        _ => 4,
    }
}

pub fn chord_name(chord: &Chord, character: &dyn Fn(u16) -> Option<char>) -> String {
    let mut keys = chord.keys().to_vec();
    keys.sort_by_key(|vk| (rank(*vk), *vk));
    keys.iter().map(|vk| key_name(*vk, character)).collect::<Vec<String>>().join(" + ")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bindings {
    pub talk: Vec<Chord>,
    pub whisper: Vec<Chord>,
    pub reply: Chord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    pub talk: bool,
    pub lane: u8,
}

pub fn evaluate(bindings: &Bindings, down: &dyn Fn(u16) -> bool) -> Held {
    let talk = bindings.talk.iter().any(|chord| chord.is_down(down));
    let mut best: Option<(usize, u8)> = None;
    for (index, chord) in bindings.whisper.iter().enumerate().take(MAX_WHISPER_KEYS) {
        if chord.is_down(down) && best.map_or(true, |(size, _)| chord.keys().len() > size) {
            best = Some((chord.keys().len(), index as u8 + 1));
        }
    }
    if bindings.reply.is_down(down) && best.map_or(true, |(size, _)| bindings.reply.keys().len() > size) {
        best = Some((bindings.reply.keys().len(), REPLY_LANE));
    }
    Held { talk, lane: best.map_or(0, |(_, lane)| lane) }
}

#[derive(Debug, Default)]
pub struct Latch {
    talk: bool,
    talk_until: Option<Instant>,
    lane: u8,
    lane_until: Option<Instant>,
}

impl Latch {
    pub fn update(&mut self, now: Instant, raw: Held, delay: Duration) -> Held {
        if raw.lane != 0 {
            self.lane = raw.lane;
            self.lane_until = None;
        } else if self.lane != 0 {
            let until = *self.lane_until.get_or_insert(now + delay);
            if now >= until {
                self.lane = 0;
                self.lane_until = None;
            }
        }
        if raw.talk {
            self.talk = true;
            self.talk_until = None;
        } else if self.talk {
            let until = *self.talk_until.get_or_insert(now + delay);
            if now >= until {
                self.talk = false;
                self.talk_until = None;
            }
        }
        Held { talk: self.talk, lane: self.lane }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureStep {
    Waiting,
    Done(Chord),
    Cancelled,
}

#[derive(Debug, Default)]
pub struct Capture {
    armed: bool,
    seen: Vec<u16>,
}

impl Capture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, down: &[u16]) -> CaptureStep {
        if !self.armed {
            self.armed = down.is_empty();
            return CaptureStep::Waiting;
        }
        if down.is_empty() {
            return if self.seen.is_empty() { CaptureStep::Waiting } else { CaptureStep::Done(Chord::new(&self.seen)) };
        }
        if self.seen.is_empty() && down.len() == 1 && down[0] == ESCAPE {
            return CaptureStep::Cancelled;
        }
        for vk in down {
            if !self.seen.contains(vk) {
                self.seen.push(*vk);
            }
        }
        CaptureStep::Waiting
    }
}
```

- [x] **Step 4:** Run `cargo test -p ps-app hotkeys`; expect 5 pass.

### Task 2: The key watcher

**Files:** Create `ps-app/src/keywatch.rs`; modify `ps-app/src/main.rs` (`mod keywatch;`), `ps-app/src/platform.rs`, `ps-voice/src/state.rs`, `ps-voice/src/lib.rs`.

**Interfaces:** Consumes Task 1. Produces
in `ps-voice`: `pub const LANES: usize = 16;`, `Shared::set_keys(&self, talk: bool, lane: u8)` (stores `ptt` and the held whisper lane; a lane of 16 or more is stored as 0), `Shared::whisper_lane(&self) -> u8`; `AudioEngine::set_ptt` is removed;
in `platform.rs`: `pub fn key_char(vk: u16) -> Option<char>` (`MapVirtualKeyW(vk, 2)`, low 16 bits, `None` for 0; a stub returning `None` off Windows), and `key_down` keeps its signature;
in `keywatch.rs`: `pub struct WatchState` with `set_bindings(&self, Bindings)`, `set_release_delay(&self, ms: u32)`, `begin_capture(&self)`, `cancel_capture(&self)`, `take_captured(&self) -> Option<CaptureStep>`; `pub struct WatchCore` with `step(&mut self, state: &WatchState, now: Instant, down: &dyn Fn(u16) -> bool) -> Held`; `pub struct KeyWatcher` with `start(audio: Arc<ps_voice::Shared>) -> Self` and `state(&self) -> &WatchState`, which stops and joins its thread when dropped.

- [x] **Step 1: Write the failing tests.** In `keywatch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::{Bindings, CaptureStep, Chord, Held};

    #[test]
    fn the_watcher_follows_the_keys_and_sends_nothing_while_choosing_one() {
        let state = WatchState::default();
        state.set_bindings(Bindings {
            talk: vec![Chord::new(&[0x87])],
            whisper: vec![Chord::new(&[0x86])],
            reply: Chord::default(),
        });
        let mut core = WatchCore::default();
        let now = Instant::now();
        assert_eq!(core.step(&state, now, &|vk| vk == 0x87), Held { talk: true, lane: 0 });
        assert_eq!(core.step(&state, now, &|vk| vk == 0x86), Held { talk: false, lane: 1 });
        state.begin_capture();
        assert_eq!(core.step(&state, now, &|vk| vk == 0x87), Held::default());
        assert_eq!(core.step(&state, now, &|_| false), Held::default());
        assert_eq!(core.step(&state, now, &|vk| vk == 0x41), Held::default());
        assert_eq!(state.take_captured(), None);
        assert_eq!(core.step(&state, now, &|_| false), Held::default());
        assert_eq!(state.take_captured(), Some(CaptureStep::Done(Chord::new(&[0x41]))));
        assert_eq!(state.take_captured(), None);
        assert_eq!(core.step(&state, now, &|vk| vk == 0x87), Held { talk: true, lane: 0 });

        state.begin_capture();
        assert_eq!(core.step(&state, now, &|_| false), Held::default());
        state.cancel_capture();
        assert_eq!(state.take_captured(), None);
        assert_eq!(core.step(&state, now, &|vk| vk == 0x87), Held { talk: true, lane: 0 });
    }
}
```

  In the `tests` module of `ps-voice/src/state.rs`:

```rust
#[test]
fn held_keys_are_stored_for_the_transmitter() {
    let s = Shared::default();
    assert_eq!(s.whisper_lane(), 0);
    s.set_keys(true, 3);
    assert!(s.ptt.load(Ordering::Relaxed));
    assert_eq!(s.whisper_lane(), 3);
    s.set_keys(false, 200);
    assert!(!s.ptt.load(Ordering::Relaxed));
    assert_eq!(s.whisper_lane(), 0);
}
```

- [x] **Step 2:** Run `cargo test -p ps-app keywatch` and `cargo test -p ps-voice held_keys`; expect compile failures.
- [x] **Step 3: Implement.** `Shared` gets `whisper_lane: AtomicU8`. In `keywatch.rs`:

```rust
#[derive(Default)]
pub struct WatchState {
    bindings: Mutex<Bindings>,
    delay_ms: AtomicU32,
    capturing: AtomicBool,
    captured: Mutex<Option<CaptureStep>>,
    stop: AtomicBool,
}

#[derive(Default)]
pub struct WatchCore {
    latch: Latch,
    capture: Option<Capture>,
}

impl WatchCore {
    pub fn step(&mut self, state: &WatchState, now: Instant, down: &dyn Fn(u16) -> bool) -> Held {
        if state.capturing.load(Ordering::Relaxed) {
            let keys: Vec<u16> = (3u16..=254).filter(|vk| usable(*vk) && down(*vk)).collect();
            let step = self.capture.get_or_insert_with(Capture::new).feed(&keys);
            if step != CaptureStep::Waiting {
                if let Ok(mut slot) = state.captured.lock() {
                    *slot = Some(step);
                }
                state.capturing.store(false, Ordering::Relaxed);
                self.capture = None;
            }
            self.latch = Latch::default();
            return Held::default();
        }
        self.capture = None;
        let raw = match state.bindings.lock() {
            Ok(bindings) => evaluate(&bindings, down),
            Err(_) => Held::default(),
        };
        self.latch.update(now, raw, Duration::from_millis(u64::from(state.delay_ms.load(Ordering::Relaxed))))
    }
}
```

  `begin_capture` clears `captured` and sets `capturing`; `cancel_capture` clears both. The thread, named `ps-keys`, loops until `stop`: `let held = core.step(&state, Instant::now(), &|vk| platform::key_down(i32::from(vk)));`, `audio.set_keys(held.talk, held.lane);`, `sleep(5 ms)`; on exit it calls `audio.set_keys(false, 0)`.
- [x] **Step 4:** In `app.rs`, `App` owns a `KeyWatcher` started after the audio engine, and the two lines in `tick` that read `PTT_KEYS` and call `set_ptt` are deleted. Until Task 3 the bindings are built from the old setting: `platform::PTT_KEYS[settings.ptt_key]` as a one-key talk chord.
- [x] **Step 5:** Run `cargo test --workspace`; expect all pass (2 new). `cargo build --workspace --all-targets`; expect no warnings.

### Task 3: Talk keys in settings, and the first live check

**Files:** Modify `ps-app/src/settings.rs`, `ps-app/src/platform.rs`, `ps-app/src/app.rs`, `ps-app/src/main.rs`, `ps-app/ui/settings.slint`, `ps-app/ui/widgets.slint`; create `tools/hold_keys.py`.

**Interfaces:** Consumes Tasks 1 and 2. Produces `Settings::talk_keys: Vec<Chord>`, `Settings::talk_release_ms: u32` (0 to 1000), and removes `Settings::ptt_key` and `platform::PTT_KEYS`. In Slint: a `KeyChip` widget (`text`, `listening`, `removable`, callbacks `clicked`, `removed`), and on `SettingsWindow`: `in property <[string]> talk-keys;`, `in property <int> listening: -1;` (index of the talk key being chosen; 50 is "a new talk key"), `in-out property <float> talk-release;`, callbacks `talk-key-change(int)`, `talk-key-add()`, `talk-key-remove(int)`, `shortcut-changed()`; `ptt-keys` and `ptt-key-index` are removed.

- [x] **Step 1: Write the failing test** in `settings.rs`, and change `round_trip` to set `talk_keys = vec![Chord::new(&[0xA4])]` and `talk_release_ms = 150` where it set `ptt_key`:

```rust
#[test]
fn old_talk_key_setting_is_carried_over() {
    let old = Settings::parse("tx_mode=1\nptt_key=8\n");
    assert_eq!(old.talk_keys, vec![Chord::new(&[0x05])]);
    assert!(Settings::parse("ptt_key=0\n").talk_keys.is_empty());
    assert!(Settings::parse("ptt_key=99\n").talk_keys.is_empty());
    let new = Settings::parse("ptt_key=8\ntalk_key=162+65\ntalk_key=135\ntalk_key=\ntalk_release_ms=250\n");
    assert_eq!(new.talk_keys, vec![Chord::new(&[0xA2, 0x41]), Chord::new(&[0x87])]);
    assert_eq!(new.talk_release_ms, 250);
    let text = new.serialize();
    assert!(text.contains("talk_key=65+162\n") && text.contains("talk_key=135\n") && !text.contains("ptt_key"));
    assert_eq!(Settings::parse(&text), new);
    assert_eq!(Settings::parse("talk_release_ms=99999\n").talk_release_ms, 1000);
    assert_eq!(Settings::default().talk_release_ms, 0);
}
```

- [x] **Step 2:** Run `cargo test -p ps-app settings`; expect a compile failure.
- [x] **Step 3: Implement.** The seventeen key codes of today's list move into `settings.rs` as a private `LEGACY_TALK_KEYS: [u16; 17]` in their current order (index 0 is "no key"). `parse` collects every non-empty `talk_key=` line; if there was none and `ptt_key` named a valid index above 0, that one key becomes the talk key. `serialize` writes one `talk_key=` line per key and never `ptt_key`. The dock wording becomes "Sends while I hold Left Ctrl" for one key, "Sends while I hold Left Ctrl or Mouse 4" for two, "Sends while I hold one of 3 keys" for more, and "Choose a talk key in settings" for none, using `chord_name` with `platform::key_char`.
- [x] **Step 4: The Shortcuts tab.** Replace the dropdown with a row of `KeyChip`s (34 px high, the key name, a small remove button, amber focus ring and the text "Press a key…" while `listening`), an "Add a key" button, and the release slider (`Slide`, 0 to 1000, step 50, read-out "0.2 s"). The hint under the chips: "Click a key, then press the key, mouse button or combination you want. Esc cancels. Works while PhishSpeak is in the background." Rust side: `talk-key-change(i)` and `talk-key-add()` call `begin_capture` and remember what is being chosen; the 33 ms tick calls `take_captured`: `Done(chord)` stores it (a combination already used for another talk key is refused with a note under the chips: "That key is already a talk key."), `Cancelled` changes nothing; either way `listening` returns to -1 and the bindings are sent to the watcher. Closing the settings window, pressing Done or changing tab calls `cancel_capture`.
- [x] **Step 5: Write `tools/hold_keys.py`**, which can press only keys no keyboard has:

```python
import ctypes
import sys
import time

ALLOWED = {f"F{number}": 0x7C + number - 13 for number in range(13, 25)}


def main():
    if len(sys.argv) != 3 or any(name not in ALLOWED for name in sys.argv[1].split("+")):
        print("usage: hold_keys.py F13..F24[+F13..F24] <seconds>")
        sys.exit(2)
    codes = [ALLOWED[name] for name in sys.argv[1].split("+")]
    user32 = ctypes.WinDLL("user32")
    for code in codes:
        user32.keybd_event(code, 0, 0, 0)
    time.sleep(float(sys.argv[2]))
    for code in reversed(codes):
        user32.keybd_event(code, 0, 2, 0)
    print(f"held {sys.argv[1]} for {sys.argv[2]} s")


main()
```

- [x] **Step 6:** `cargo test --workspace` and `cargo build --workspace --all-targets`; expect all pass (1 new, `hotkey_table_is_sane` reduced to its two `key_down` assertions) and no warnings.
- [x] **Step 7: Live check, the old setting.** Throwaway profile with `tx_mode=1` and `ptt_key=14` (F8 in the old list) and no `talk_key`. Start the app; the dock must read "Sends while I hold F8"; close it; the file must now contain `talk_key=119` and no `ptt_key`.
- [x] **Step 8: Live check, a talk key.** Profile with `tx_mode=1`, `talk_key=135` (F24), output volume 0. Test server up, `channeltest` listening in Lobby, the app connected there. Run `python tools/hold_keys.py F24 2`. Expected from the listener: a first voice packet within 60 ms of the press, about 100 packets, then an end-of-talk packet. Repeat with `talk_release_ms=300`: about 115 packets. Repeat with `talk_key=134+135` and `hold_keys.py F23+F24 2`: about 100 packets; `hold_keys.py F24 2` alone: none.
- [x] **Step 9: Live check, choosing a key.** With the software renderer, open Settings, Shortcuts, click "Add a key", run `hold_keys.py F23 0.3`, and take a screenshot: a chip named "F23". Click it again and close the settings window without pressing anything; run `hold_keys.py F24 2` with a listener: voice must flow, proving nothing was left waiting for a key.

## Part B: whisper keys

### Task 4: Whispers on the wire

**Files:** Modify `ps-protocol/src/voice.rs`, `ps-client/src/lib.rs`, `ps-client/src/conn.rs`, `ps-client/src/book.rs`, `ps-app/src/session.rs` (the `Event::Talking` pattern), `ps-voice/examples/voicetest.rs` (the same), `ps-voice/examples/channeltest.rs`; create `ps-client/examples/whispertest.rs`, `tools/seed_whisper_tree.py`.

**Interfaces:** Produces
in `ps-protocol::voice`: `pub const GROUP_WHISPER_HEADER_LEN: usize = 13;`, `pub fn whisper_header_len(channels: usize, clients: usize) -> usize`, `pub fn encode_c2s_whisper(codec: u8, channels: &[u64], clients: &[u16], data: &[u8]) -> Option<Vec<u8>>` (`None` when either list is longer than 255), `pub fn encode_c2s_group_whisper(codec: u8, who: u8, scope: u8, id: u64, data: &[u8]) -> Vec<u8>`; both leave the first two bytes zero for the voice id;
in `ps-client`: `pub const ERROR_NO_WHISPER_TARGETS: u32 = 0x070c;`,
`pub enum WhisperGroup { ServerGroup(u64), ChannelGroup(u64), Commanders, Everyone }`,
`pub enum WhisperScope { AllChannels, CurrentChannel, ParentChannel, AllParentChannels, ChannelFamily, WholeFamily, Subchannels }`,
`pub enum WhisperTarget { List { channels: Vec<u64>, clients: Vec<u16> }, Group { who: WhisperGroup, scope: WhisperScope } }` with `header_len(&self) -> usize`, `frame_room(&self) -> usize` (`MAX_C2S_PAYLOAD` minus the header, 0 if negative), `is_group(&self) -> bool`, `payload(&self, codec: u8, data: &[u8]) -> Option<Vec<u8>>`,
`ClientHandle::send_whisper(&self, target: &WhisperTarget, codec: u8, data: &[u8])`, `ClientHandle::set_channel_commander(&self, on: bool)`,
`Event::Talking { client_id: u16, talking: bool, whisper: bool }`, `ClientInfo::whispering: bool`.

- [x] **Step 1: Write the failing tests.** In `ps-protocol/src/voice.rs`:

```rust
#[test]
fn whisper_to_a_list_matches_the_wire_layout() {
    let raw = encode_c2s_whisper(CODEC_OPUS_VOICE, &[1, 0x0102030405060708], &[9, 0x0A0B], &[0xEE, 0xFF]).unwrap();
    assert_eq!(
        raw,
        vec![0, 0, 4, 2, 2, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8, 0, 9, 0x0A, 0x0B, 0xEE, 0xFF]
    );
    assert_eq!(whisper_header_len(2, 2), 25);
    assert_eq!(whisper_header_len(30, 60), 365);
    assert_eq!(encode_c2s_whisper(CODEC_OPUS_VOICE, &[7], &[], &[]).unwrap().len(), 13);
    assert_eq!(encode_c2s_whisper(CODEC_OPUS_VOICE, &[], &[], &[5]).unwrap(), vec![0, 0, 4, 0, 0, 5]);
    assert!(encode_c2s_whisper(4, &vec![1; 256], &[], &[]).is_none());
    assert!(encode_c2s_whisper(4, &[], &vec![1; 256], &[]).is_none());
}

#[test]
fn whisper_to_a_group_matches_the_wire_layout() {
    assert_eq!(
        encode_c2s_group_whisper(CODEC_OPUS_VOICE, 2, 5, 0, &[0xEE]),
        vec![0, 0, 4, 2, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0xEE]
    );
    assert_eq!(
        encode_c2s_group_whisper(CODEC_OPUS_MUSIC, 0, 0, 0x0102030405060708, &[]),
        vec![0, 0, 5, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(GROUP_WHISPER_HEADER_LEN, 13);
}
```

  In a `tests` module at the end of `ps-client/src/lib.rs`:

```rust
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
```

  In the `tests` module of `book.rs`:

```rust
#[test]
fn whispers_are_marked_on_the_person() {
    let mut book = book_from("channellist cid=1 cpid=0 channel_order=0 channel_name=Lobby");
    book.clients_entered(&Command::parse("notifycliententerview cfid=0 ctid=1 reasonid=0 clid=4 client_nickname=Marlin client_type=0"));
    assert!(book.set_talking(4, true, true));
    assert!(book.clients[&4].talking && book.clients[&4].whispering);
    assert!(!book.set_talking(4, true, true));
    assert!(book.set_talking(4, true, false));
    assert!(!book.clients[&4].whispering);
    assert!(book.set_talking(4, false, false));
    assert!(!book.clients[&4].talking && !book.clients[&4].whispering);
}
```

- [x] **Step 2:** Run `cargo test -p ps-protocol whisper` and `cargo test -p ps-client whisper`; expect compile failures.
- [x] **Step 3: Implement.** The two encoders write exactly the layouts in "Facts". `WhisperGroup` maps to `(0, id)`, `(1, id)`, `(2, 0)`, `(3, 0)`; `WhisperScope` to 0 through 6 in declaration order. `Book::set_talking(id, talking, whisper)` returns true when either flag changes and clears `whispering` when talking stops; the existing call in `client_lifecycle` gains `false`. In `conn.rs`: `on_voice` passes `ptype == PacketType::VoiceWhisper` to `set_talking`, which emits `Event::Talking { client_id, talking, whisper }`; the talk timeout passes `false`; a new `Request::Whisper { payload: Vec<u8>, group: bool }` is sent with `self.send_packet(PacketType::VoiceWhisper, if group { FLAG_NEWPROTOCOL } else { 0 }, &payload)` when connected and the payload is at most `MAX_C2S_PAYLOAD` bytes. `send_whisper` builds the payload with `target.payload` and sends nothing when that is `None`. `set_channel_commander` sends `clientupdate client_is_channel_commander=0|1`. Every existing match on `Event::Talking` gains `..` or the new field.
- [x] **Step 4:** Run `cargo test --workspace`; expect all pass (4 new).
- [x] **Step 5: `channeltest` says what it heard.** Its sink keeps separate counts for talk and whisper and prints "first whisper packet from client N" or "first voice packet from client N", "end-of-talk" or "end-of-whisper", and both counts in the per-second and total lines.
- [x] **Step 6: Write `tools/seed_whisper_tree.py`**: over ServerQuery, with `--password`, look up channels by name, create `Booth` under `Radio` and `Drift` under `Deep Rock` if missing (`channelcreate channel_name=Booth channel_flag_permanent=1 cpid=<Radio>`), grant guests the commander permission (`servergroupaddperm sgid=<Guest, the one with type=1> permsid=b_client_use_channel_commander permvalue=1 permnegated=0 permskip=0`), and print `--booth <cid> --drift <cid>`. These exact commands were used on 2026-10-06.
- [x] **Step 7: Write `ps-client/examples/whispertest.rs`**, which re-runs the table from "Facts" and fails loudly if a server behaves differently:

```rust
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ps_client::{ClientHandle, ConnectOptions, Event, VoiceSink, WhisperGroup, WhisperScope, WhisperTarget};
use ps_identity::Identity;

struct Peer {
    name: &'static str,
    handle: ClientHandle,
    events: Receiver<Event>,
    heard: Arc<Mutex<(u32, u32)>>,
    id: u16,
}

fn connect(host: &str, name: &'static str) -> Peer {
    let heard = Arc::new(Mutex::new((0u32, 0u32)));
    let counts = heard.clone();
    let sink: VoiceSink = Box::new(move |packet| {
        if let (Ok(mut seen), false) = (counts.lock(), packet.data.is_empty()) {
            if packet.whisper {
                seen.1 += 1;
            } else {
                seen.0 += 1;
            }
        }
    });
    let mut options = ConnectOptions::new(host, ps_client::DEFAULT_PORT, Identity::generate(name, name));
    options.nickname = name.to_string();
    let (tx, rx) = mpsc::channel();
    Peer { name, handle: ClientHandle::connect(options, tx, Some(sink)), events: rx, heard, id: 0 }
}

fn settle(peers: &mut [Peer], time: Duration) {
    let until = Instant::now() + time;
    while Instant::now() < until {
        for peer in peers.iter_mut() {
            while let Ok(event) = peer.events.try_recv() {
                if let Event::Connected { client_id, .. } = event {
                    peer.id = client_id;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn check(peers: &mut [Peer], label: &str, target: Option<&WhisperTarget>, expect: &[&str]) -> bool {
    for peer in peers.iter() {
        *peer.heard.lock().unwrap() = (0, 0);
    }
    for round in 0..16 {
        let data: &[u8] = if round < 15 { &[0x55; 30] } else { &[] };
        match target {
            Some(target) => peers[0].handle.send_whisper(target, 4, data),
            None => peers[0].handle.send_voice(4, data),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    settle(peers, Duration::from_millis(300));
    let got: Vec<&str> = peers
        .iter()
        .skip(1)
        .filter(|peer| {
            let seen = *peer.heard.lock().unwrap();
            if target.is_some() { seen.1 > 0 && seen.0 == 0 } else { seen.0 > 0 && seen.1 == 0 }
        })
        .map(|peer| peer.name)
        .collect();
    let ok = got == expect;
    println!("{} {label}: {got:?}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |name: &str| -> u64 {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(0)
    };
    if args.is_empty() || number("--booth") == 0 || number("--drift") == 0 {
        eprintln!("usage: whispertest <host> --booth CID --drift CID   (run tools/seed_whisper_tree.py first)");
        std::process::exit(2);
    }
    let (lobby, deep, radio, booth, drift) = (1u64, 2u64, 4u64, number("--booth"), number("--drift"));
    let plan = [("Sender", radio), ("InLobby", lobby), ("InDeep", deep), ("InRadio", radio), ("InBooth", booth), ("InDrift", drift)];
    let mut peers: Vec<Peer> = Vec::new();
    for (name, _) in plan.iter() {
        peers.push(connect(&args[0], name));
        settle(&mut peers, Duration::from_millis(400));
    }
    settle(&mut peers, Duration::from_secs(2));
    for (index, (_, channel)) in plan.iter().enumerate() {
        if *channel != lobby {
            peers[index].handle.join_channel(*channel, "");
        }
    }
    settle(&mut peers, Duration::from_millis(1200));
    let drift_id = peers[5].id;
    let everyone = |scope| WhisperTarget::Group { who: WhisperGroup::Everyone, scope };
    let list = |channels: Vec<u64>, clients: Vec<u16>| WhisperTarget::List { channels, clients };
    let mut ok = true;
    ok &= check(&mut peers, "ordinary talk", None, &["InRadio"]);
    ok &= check(&mut peers, "list: Lobby", Some(&list(vec![lobby], vec![])), &["InLobby"]);
    ok &= check(&mut peers, "list: one person", Some(&list(vec![], vec![drift_id])), &["InDrift"]);
    ok &= check(&mut peers, "list: empty", Some(&list(vec![], vec![])), &[]);
    ok &= check(&mut peers, "everyone, everywhere", Some(&everyone(WhisperScope::AllChannels)), &["InLobby", "InDeep", "InRadio", "InBooth", "InDrift"]);
    ok &= check(&mut peers, "everyone, my channel", Some(&everyone(WhisperScope::CurrentChannel)), &["InRadio"]);
    ok &= check(&mut peers, "everyone, the channel above", Some(&everyone(WhisperScope::ParentChannel)), &["InDeep"]);
    ok &= check(&mut peers, "everyone, every channel above", Some(&everyone(WhisperScope::AllParentChannels)), &["InDeep"]);
    ok &= check(&mut peers, "everyone, my channel and below", Some(&everyone(WhisperScope::ChannelFamily)), &["InRadio", "InBooth"]);
    ok &= check(&mut peers, "everyone, my whole branch", Some(&everyone(WhisperScope::WholeFamily)), &["InDeep", "InRadio", "InBooth", "InDrift"]);
    ok &= check(&mut peers, "everyone, right below", Some(&everyone(WhisperScope::Subchannels)), &["InBooth"]);
    let commanders = WhisperTarget::Group { who: WhisperGroup::Commanders, scope: WhisperScope::AllChannels };
    ok &= check(&mut peers, "commanders, everywhere (there are none)", Some(&commanders), &[]);
    for peer in peers.iter() {
        peer.handle.disconnect("whisper test finished");
    }
    settle(&mut peers, Duration::from_millis(1000));
    std::process::exit(if ok { 0 } else { 1 });
}
```

- [x] **Step 8: Live check.** Test server up, `python3 tools/seed_whisper_tree.py --password <the test server's query password>` inside WSL, then `cargo run -p ps-client --example whispertest -- <server ip> --booth <cid> --drift <cid>`. Expected: twelve `PASS` lines and exit code 0.

### Task 5: Lanes in the audio engine

**Files:** Modify `ps-voice/src/state.rs`, `ps-voice/src/capture.rs`, `ps-voice/src/lib.rs`, `ps-app/src/app.rs` (the frame sink closure takes a lane and, until Task 7, sends only lane 0).

**Interfaces:** Consumes `Shared::set_keys` and `whisper_lane` from Task 2. Produces
`pub type FrameSink = Box<dyn FnMut(u8, u8, &[u8]) + Send>;` (lane, codec, data),
`Shared::set_lane_room(&self, lane: u8, bytes: usize)` (clamped to 24..=`MAX_PACKET_BYTES`), `Shared::lane_room(&self, lane: u8) -> usize` (`MAX_PACKET_BYTES` until set), `Shared::on_air_lane(&self) -> Option<u8>` (the lane frames are going out on right now), `Shared::set_on_air(&self, lane: Option<u8>)` (crate-internal, used by the transmitter),
and `Transmitter::process(&mut self, input: &[f32], shared: &Shared, sink: &mut dyn FnMut(u8, u8, &[u8]))`.

- [x] **Step 1: Write the failing tests** in the `tests` module of `capture.rs`. Change the existing `collect` helper to take the three-argument closure and drop the lane, then add:

```rust
fn lanes(tx: &mut Transmitter, shared: &Shared, input: &[f32]) -> Vec<(u8, u8, usize)> {
    let mut out = Vec::new();
    tx.process(input, shared, &mut |lane, codec, data| out.push((lane, codec, data.len())));
    out
}

fn shape(frames: &[(u8, u8, usize)]) -> Vec<(u8, bool)> {
    frames.iter().map(|frame| (frame.0, frame.2 > 0)).collect()
}

#[test]
fn a_whisper_key_opens_the_microphone_on_its_own_lane() {
    let shared = Shared::default();
    shared.tx_enabled.store(true, Ordering::Relaxed);
    let mut tx = Transmitter::new(48_000);
    let silence = vec![0.0f32; 960 * 3];
    assert!(lanes(&mut tx, &shared, &silence).is_empty());
    assert_eq!(shared.on_air_lane(), None);
    shared.set_keys(false, 2);
    let frames = lanes(&mut tx, &shared, &silence);
    assert_eq!(shape(&frames), vec![(2, true), (2, true), (2, true)]);
    assert!(frames.iter().all(|frame| frame.1 == CODEC_OPUS_VOICE));
    assert_eq!(shared.on_air_lane(), Some(2));
    shared.set_keys(false, 0);
    assert_eq!(shape(&lanes(&mut tx, &shared, &silence)), vec![(2, false)]);
    assert_eq!(shared.on_air_lane(), None);
}

#[test]
fn changing_lane_mid_sentence_ends_the_old_one_first() {
    let shared = Shared::default();
    shared.set_tx_mode(TxMode::PushToTalk);
    shared.tx_enabled.store(true, Ordering::Relaxed);
    let mut tx = Transmitter::new(48_000);
    let two = vec![0.0f32; 960 * 2];
    shared.set_keys(true, 0);
    assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(0, true), (0, true)]);
    shared.set_keys(true, 1);
    assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(0, false), (1, true), (1, true)]);
    shared.set_keys(true, 5);
    assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(1, false), (5, true), (5, true)]);
    shared.set_keys(true, 0);
    assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(5, false), (0, true), (0, true)]);
    shared.set_keys(false, 0);
    assert_eq!(shape(&lanes(&mut tx, &shared, &two)), vec![(0, false)]);
    assert!(lanes(&mut tx, &shared, &two).is_empty());
}

#[test]
fn whisper_frames_fit_their_lane_and_use_the_voice_codec() {
    let shared = Shared::default();
    shared.tx_enabled.store(true, Ordering::Relaxed);
    shared.codec.store(CODEC_OPUS_MUSIC, Ordering::Relaxed);
    shared.codec_quality.store(10, Ordering::Relaxed);
    assert_eq!(shared.lane_room(3), MAX_PACKET_BYTES);
    shared.set_lane_room(3, 40);
    shared.set_lane_room(4, 1);
    assert_eq!((shared.lane_room(3), shared.lane_room(4)), (40, 24));
    shared.set_keys(false, 3);
    let mut tx = Transmitter::new(48_000);
    let frames = lanes(&mut tx, &shared, &tone(10, 0.5, 48_000, 0));
    assert_eq!(frames.len(), 10);
    assert!(frames.iter().all(|frame| frame.0 == 3 && frame.1 == CODEC_OPUS_VOICE && frame.2 > 0 && frame.2 <= 40));
    shared.set_keys(false, 0);
    shared.set_tx_mode(TxMode::Continuous);
    let talk = lanes(&mut tx, &shared, &tone(3, 0.5, 48_000, 0));
    assert_eq!(talk[0], (3, CODEC_OPUS_VOICE, 0));
    assert!(talk[1..].iter().all(|frame| frame.0 == 0 && frame.1 == CODEC_OPUS_MUSIC && frame.2 > 0));
}

#[test]
fn muting_stops_whispers_too() {
    let shared = Shared::default();
    shared.tx_enabled.store(true, Ordering::Relaxed);
    shared.set_keys(false, 1);
    let mut tx = Transmitter::new(48_000);
    let one = vec![0.0f32; 960];
    assert_eq!(shape(&lanes(&mut tx, &shared, &one)), vec![(1, true)]);
    shared.mic_muted.store(true, Ordering::Relaxed);
    assert_eq!(shape(&lanes(&mut tx, &shared, &one)), vec![(1, false)]);
    assert!(lanes(&mut tx, &shared, &one).is_empty());
    shared.mic_muted.store(false, Ordering::Relaxed);
    shared.tx_enabled.store(false, Ordering::Relaxed);
    assert!(lanes(&mut tx, &shared, &one).is_empty());
}
```

- [x] **Step 2:** Run `cargo test -p ps-voice`; expect compile failures.
- [x] **Step 3: Implement.** `Shared` gets `lane_room: [AtomicU16; LANES]` (0 meaning unset) and `on_air: AtomicU8` (255 meaning none). `Transmitter` gets a `lane: u8` field. The decision part of `process_frame` becomes:

```rust
let lane = shared.whisper_lane();
let mode = shared.tx_mode();
let gate_open = lane != 0
    || match mode {
        TxMode::Continuous => true,
        TxMode::PushToTalk => shared.ptt.load(Ordering::Relaxed),
        TxMode::VoiceActivation => {
            if level >= shared.vad_threshold() {
                self.hangover = HANGOVER_FRAMES;
                true
            } else if self.hangover > 0 {
                self.hangover -= 1;
                true
            } else {
                false
            }
        }
    };
let allowed = shared.tx_enabled.load(Ordering::Relaxed)
    && !shared.mic_muted.load(Ordering::Relaxed)
    && !shared.speaker_muted.load(Ordering::Relaxed);
let active = gate_open && allowed;

if self.transmitting && (!active || lane != self.lane) {
    self.transmitting = false;
    self.hangover = 0;
    let ended = if self.codec == 0 { CODEC_OPUS_VOICE } else { self.codec };
    sink(self.lane, ended, &[]);
}
if active {
    let codec = if lane == 0 { shared.codec.load(Ordering::Relaxed) } else { CODEC_OPUS_VOICE };
    let quality = shared.codec_quality.load(Ordering::Relaxed);
    if !self.ensure_encoder(codec, quality) {
        shared.transmitting.store(false, Ordering::Relaxed);
        shared.set_on_air(None);
        return;
    }
    if !self.transmitting {
        self.transmitting = true;
        self.lane = lane;
        if let Some(encoder) = self.encoder.as_mut() {
            encoder.reset();
        }
        if lane == 0 && mode == TxMode::VoiceActivation {
            let earlier: Vec<Vec<f32>> = self.lookback.drain(..).collect();
            for old in &earlier {
                self.encode_and_emit(old, 0, MAX_PACKET_BYTES, sink);
            }
        }
    }
    self.lookback.clear();
    let room = if lane == 0 { MAX_PACKET_BYTES } else { shared.lane_room(lane) };
    self.encode_and_emit(frame, lane, room, sink);
} else {
    self.lookback.push_back(frame.to_vec());
    while self.lookback.len() > LOOKBACK_FRAMES {
        self.lookback.pop_front();
    }
}
shared.transmitting.store(active, Ordering::Relaxed);
shared.set_on_air(if active { Some(lane) } else { None });
```

  `encode_and_emit(frame, lane, room, sink)` encodes into `&mut self.packet[..room]` and calls `sink(lane, codec, &self.packet[..n])`. One thing the first test relies on: in voice-activation mode a held whisper key must not disturb `hangover`, so the `lane != 0` test comes first and short-circuits. In `lib.rs` the transmit loop passes the lane through to the sink; the microphone test (loopback) plays back every lane. In `app.rs` the sink becomes `move |lane, codec, data| if lane == 0 { client.send_voice(codec, data) }`, which already satisfies "a whisper key never sends to the channel" before whisper keys exist.
- [x] **Step 4:** Run `cargo test --workspace`; expect all pass (4 new) and the six existing transmitter tests unchanged in what they assert. `cargo build --workspace --all-targets`; expect no warnings.

### Task 6: Whisper keys

**Files:** Create `ps-app/src/whisper.rs`; modify `ps-app/src/main.rs` (`mod whisper;`).

**Interfaces:** Consumes `Chord` (Task 1), `WhisperTarget`, `WhisperGroup`, `WhisperScope` (Task 4), `ServerView`. Produces
`pub const MAX_LIST_CHANNELS: usize = 30;`, `pub const MAX_LIST_PEOPLE: usize = 60;`,
`pub enum Who { Everyone, Commanders, ServerGroup { id: u64, name: String }, ChannelGroup { id: u64, name: String } }`,
`pub enum Aim { List { channels: Vec<(u64, String)>, people: Vec<(String, String)> }, Group { who: Who, scope: WhisperScope } }` (people are `(uid, name)`),
`pub struct WhisperKey { pub chord: Chord, pub server_uid: String, pub server_name: String, pub aim: Aim }` (`server_uid` is empty for everyone and commanders, which work on any server),
`pub struct WhisperKeys { pub items: Vec<WhisperKey> }` with `parse(&str) -> Self`, `serialize(&self) -> String`, `load() -> Self`, `save(&self) -> io::Result<()>`,
`pub enum Unusable { OtherServer, NobodyThere }`, `pub fn resolve(key: &WhisperKey, view: &ServerView) -> Result<WhisperTarget, Unusable>`,
`pub fn scope_label(scope: WhisperScope) -> &'static str`, `pub fn describe(aim: &Aim) -> String`,
`pub enum Route<'a> { Talk, Whisper(&'a WhisperTarget), Nothing }`, `pub fn route(lane: u8, table: &[Option<WhisperTarget>]) -> Route<'_>`,
`pub fn lane_table(keys: &[WhisperKey], view: Option<&ServerView>, reply_to: Option<u16>) -> Vec<Option<WhisperTarget>>` (14 entries: index 0 unused, 1 to 12 the keys in order, 13 the reply target).

- [x] **Step 1: Write the failing tests** in `whisper.rs`:

```rust
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
    }
}
```

- [x] **Step 2:** Run `cargo test -p ps-app whisper`; expect a compile failure.
- [x] **Step 3: Implement.**
  - File `whisper.ini` next to `settings.ini`: one `[whisper]` section per key with `key=` (the chord text, may be empty), `kind=` (`list`, `everyone`, `commanders`, `server_group`, `channel_group`), `scope=` (`all`, `current`, `parent`, `all_parents`, `family`, `whole_family`, `subchannels`; missing or unknown means `all`), `server=` and `server_name=`, `group=` and `group_name=`, and repeated `channel=<id> <name>` and `person=<uid> <name>` lines split at the first space. A section with an unknown `kind` is skipped; a group kind that needs an id and has none is skipped; values are written on one line as `bookmarks.rs` does.
  - `resolve`, in full because it decides who hears you:

```rust
pub fn resolve(key: &WhisperKey, view: &ServerView) -> Result<WhisperTarget, Unusable> {
    if !key.server_uid.is_empty() && key.server_uid != view.server.uid {
        return Err(Unusable::OtherServer);
    }
    match &key.aim {
        Aim::List { channels, people } => {
            let channels: Vec<u64> = channels
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| view.channels.iter().any(|node| node.channel.id == *id))
                .collect();
            let mut clients: Vec<u16> = Vec::new();
            for node in &view.channels {
                for client in &node.clients {
                    let listed = people.iter().any(|(uid, _)| !uid.is_empty() && *uid == client.uid);
                    if listed && client.id != view.own_id && !clients.contains(&client.id) {
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
```

  - `describe`: for a list, the names of channels then people; one name alone, two joined with "and", otherwise the first followed by "and N more"; "Nobody yet" when empty. For a group, the who ("Everyone", "Channel commanders", or the group's name) then a comma and `scope_label`. The seven labels: "everywhere", "my channel", "the channel above mine", "every channel above mine", "my channel and all below it", "my whole branch", "the channels right below mine".

```rust
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
```

  The `view` helper in the tests gives client 8 to Marlin, which is why the reply lane resolves there.
- [x] **Step 4:** Run `cargo test -p ps-app whisper`; expect 5 pass.

### Task 7: Wiring the app

**Files:** Modify `ps-app/src/app.rs`, `ps-app/src/session.rs`, `ps-app/src/settings.rs`.

**Interfaces:** Consumes everything above. Produces `Settings::reply_key: Chord`, `Settings::allow_whispers: bool` (default true), `RowData::whispering: bool`, `RowData::commander: bool`, `Outcome::whisper_unheard: bool`, `Outcome::whisper_from: Option<u16>`.

- [x] **Step 1: Write the failing tests.** In `session.rs`, a new test:

```rust
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
```

  In `settings.rs`, extend `old_talk_key_setting_is_carried_over` with: `reply_key=96` and `allow_whispers=0` parse and round-trip, and `Settings::default().allow_whispers` is true.
- [x] **Step 2:** Run `cargo test -p ps-app`; expect failures.
- [x] **Step 3: Implement in `session.rs`.** `Event::Talking` stores both flags on the person in the view. When a whisper starts from someone who has not whispered in the last 60 seconds, add one dim line "<name> is whispering to you"; set `whisper_from` every time a whisper starts. `ServerError` with `ERROR_NO_WHISPER_TARGETS` adds no chat line and sets `whisper_unheard`. `build_rows` copies `whispering` and `is_channel_commander` onto person rows; a whispering person's tag reads "whispers to you".
- [x] **Step 4: Implement in `app.rs`.**
  - `App` owns `WhisperKeys` (loaded at start), a lane table behind `Arc<Mutex<Vec<Option<WhisperTarget>>>>`, `reply_to: Option<(u16, u16)>` (session id, client id) and `unheard_at: Option<Instant>`.
  - The frame sink installed by `route_mic` becomes: lock the table, `match route(lane, &table) { Route::Talk => client.send_voice(codec, data), Route::Whisper(target) => client.send_whisper(target, codec, data), Route::Nothing => {} }`.
  - The table is rebuilt with `lane_table(&keys, viewed session's view, reply_to for that session)` whenever the viewed session changes, its view arrives, the keys are edited or a whisper arrives; the same moment sets `set_lane_room(lane, target.frame_room())` for every lane that has a target, and sends the whisper chords and reply chord to the key watcher. Because the sink is replaced and the table rebuilt in `route_mic` before the new session becomes the microphone's target, a whisper key pressed during a server switch can only reach the new server's table.
  - A whisper arriving (`Outcome::whisper_from`) on the viewed session sets `reply_to`; `Outcome::whisper_unheard` sets `unheard_at = now`.
  - `allow_whispers` lives in an `Arc<AtomicBool>` read by every session's voice sink: when false, packets with `packet.whisper` are not pushed to the mixer, and `whisper_from` is ignored.
  - Dock wording, checked in this order after "Not connected", "Connecting", "Sound muted" and "Microphone muted": if `shared.whisper_lane()` is not 0 and its table entry is empty, "That whisper key is for <server name>" (other server), "Nobody on that whisper key is here" (`NobodyThere`) or "Nobody has whispered to you yet" (reply lane); if `on_air_lane()` is a whisper lane and `unheard_at` is under 2 seconds old, "Nobody is there to hear that whisper"; if it is a whisper lane, "Whispering to <describe>" (for the reply lane, "Whispering to <name>"); then "Talking" and the send-mode wording as today.
  - `toggle_commander`: calls `set_channel_commander(!own.is_channel_commander)` on the viewed session. A refusal arrives as error `0x0a08`, which already reads "You do not have permission to do that here." in the chat drawer.
- [x] **Step 5:** `cargo test --workspace`; expect all pass (1 new, 1 extended). `cargo build --workspace --all-targets`; expect no warnings.

### Task 8: The Shortcuts tab and the marks

**Files:** Modify `ps-app/ui/settings.slint`, `ps-app/ui/widgets.slint`, `ps-app/ui/main.slint`, `ps-app/ui/theme.slint`, `ps-app/src/app.rs`, `ps-app/src/main.rs`; create `ps-app/ui/icons/commander.svg`.

**Interfaces:** Consumes Task 7. Produces in Slint:
`struct WhisperKeyRow { key: string, summary: string, note: string }`, `struct PickRow { person: bool, depth: int, text: string, ticked: bool, gone: bool }`;
on `SettingsWindow`: `in property <[WhisperKeyRow]> whisper-keys;`, `in property <string> reply-key;`, `in-out property <bool> allow-whispers;`, `in-out property <bool> editor-open;`, `in property <string> editor-key;`, `in-out property <int> editor-kind;` (0 channels and people, 1 a group), `in property <string> editor-server;`, `in property <[PickRow]> editor-rows;`, `in-out property <int> editor-who;` (0 everyone, 1 channel commanders, 2 a server group, 3 a channel group), `in property <[string]> editor-groups;`, `in-out property <int> editor-group;`, `in-out property <int> editor-scope;`, `in property <string> editor-note;`; `listening` gains the values 60 (reply key) and 70 (the key in the editor);
callbacks `whisper-key-add()`, `whisper-key-edit(int)`, `whisper-key-remove(int)`, `reply-key-change()`, `reply-key-clear()`, `editor-key-change()`, `editor-toggle(int)`, `editor-changed()`, `editor-save()`, `editor-cancel()`;
on `TreeRow`: `whispering: bool`, `commander: bool`; on `PhishSpeakApp`: `in property <bool> commander;`, `callback toggle-commander();`; `Icons.commander`.

- [x] **Step 1: The list.** Under the talk keys: a heading "Whisper keys", one 34 px row per key (a `KeyChip`, the summary, a dim note such as "for Reef Runners" or "no key yet", "Edit" and "Remove"), then "Add a whisper key" (disabled at twelve, with the note "Twelve is the most."). Then the reply key chip with the hint "Talks to whoever whispered to you last.", and the tick box "Let others whisper to me".
- [x] **Step 2: The editor** replaces the tab's content while `editor-open`:
  - "Whisper key": a `KeyChip`. A combination already used by a talk key or another whisper key is refused with "That key is already used for <what>."
  - "Whisper to": a `Segmented` with "Channels and people" and "A group".
  - Channels and people: the hint "From <server>. Tick the channels and the people your voice should go to." and a scrolling list of the viewed server's tree, one 28 px row per channel and, indented under it, per person, each with a tick box. Entries the key holds that are not on the server now are listed first, dimmed, marked "not here now", and can be unticked. With no server viewed: "Connect to a server to choose channels and people from it." Ticking a 31st channel or a 61st person is refused with a note.
  - A group: "Who" as a `Dropdown` (Everyone, Channel commanders, A server group, A channel group); for the last two a second `Dropdown` with the viewed server's groups by name; "Where" as a `Dropdown` with the seven places, and under it one line that says what the choice reaches, taken from the table in "What this builds".
  - "Save" (the one amber button, disabled until there is a target) and "Cancel". Saving writes `whisper.ini`.
- [x] **Step 3: The main window.** A person row with `commander` shows `Icons.commander` (a small chevron mark, 14 px, tinted Drift) left of the mute marks. The servers menu gains, while viewing a connected server, "Be a channel commander" or "Stop being a channel commander". A whispering person keeps the amber talking dot and shows the tag "whispers to you".
- [x] **Step 4:** `cargo build -p ps-app`; expect no warnings. Screenshots with the software renderer of: the Shortcuts tab with two talk keys and three whisper keys, the editor in both modes, a row with the commander mark, a row marked "whispers to you". Check each against the constraints: sentence case, 28 px controls, one amber button.

### Task 9: Live checks and documents

**Files:** Modify `PLAN.md`, `README.md`.

All checks use the test server seeded by `tools/seed_whisper_tree.py`, the software renderer, a throwaway profile with output volume 0, generated identities, `tools/hold_keys.py` for the keys, and `channeltest` listeners.

- [x] **Step 1: A group key.** The app in Radio with "When I speak" selected and a silent microphone. Whisper keys: F23 "Everyone, the channel above mine", F22 "Everyone, the channels right below mine". Listeners in Deep Rock, Radio and Booth. `hold_keys.py F23 2`: Deep Rock hears about 100 whisper packets and an end-of-whisper; Radio and Booth hear nothing. `hold_keys.py F22 2`: only Booth hears it. The dock reads "Whispering to everyone, the channel above mine" in a screenshot taken during the first.
- [x] **Step 2: A list key.** F21 aimed at Lobby and at one listener in Tide Pool, picked in the editor with clicks. `hold_keys.py F21 2`: the Lobby listener and that one listener hear whispers; a second listener in Tide Pool hears nothing.
- [x] **Step 3: Never to the channel.** With the list key from Step 2 saved, stop the Tide Pool listener, delete nothing else, and view a second connection whose server UID differs (or, with one test server, edit `whisper.ini` so the key's `server=` names another UID). Listener in the app's own channel. `hold_keys.py F21 2`: that listener hears nothing at all, and the dock reads "That whisper key is for <name>". Then aim a key at a channel id that does not exist: nothing is heard and the dock reads "Nobody on that whisper key is here".
- [x] **Step 4: Changing audience mid-sentence.** Send mode "While I hold a key", talk key F24, whisper key F23 to the parent channel. Listeners in the app's channel and in the parent. Run `hold_keys.py F24 3` and, one second in, `hold_keys.py F23 1` from a second shell. Expected: the channel listener gets about 50 voice packets, an end-of-talk, then about 50 more voice packets and an end-of-talk; the parent listener gets about 50 whisper packets and an end-of-whisper; neither gets a packet of the other kind.
- [x] **Step 5: Nobody there.** A key "Everyone, the channel above mine" pressed while the app is in a top-level channel: the dock reads "Nobody is there to hear that whisper" and the chat drawer gains no line.
- [x] **Step 6: Being whispered to.** `whispertest`-style sender whispering to the app's client: the sender's row shows the amber dot and "whispers to you", one dim chat line appears, and with the reply key bound to F20, `hold_keys.py F20 2` makes a listener running as that sender's identity hear whisper packets. Turn off "Let others whisper to me" and repeat: no mark, no line, and the mixer's talker count for that client stays at zero (visible as no talking dot).
- [x] **Step 7: Channel commander.** With the permission granted by the seed script, the menu entry sets the mark on the app's own row; a key "Channel commanders, everywhere" from a second client reaches the app and nobody else. Remove the permission through ServerQuery and try again: the chat drawer shows "You do not have permission to do that here." and no mark appears.
- [x] **Step 8: From a music channel, with a long list.** The app in Radio (Opus Music), a list key with 30 channels (29 of them ids that do not exist, written into `whisper.ini`) and Lobby: the Lobby listener hears whisper packets, each of codec 4.
- [ ] **Step 9: One thing only you can do.** With the official TeamSpeak client on the same server: whisper from PhishSpeak to it and from it to PhishSpeak, and say what you heard. Everything above is PhishSpeak talking to PhishSpeak.
- [x] **Step 10: Documents.** In `PLAN.md`: a "Whisper" part in the protocol notes with the packet layouts, the who-hears-what table and the `0x070c` answer; the key-reading approach and its limits under "Known issues / decisions"; status, crates table, dev tools and test count. In `README.md`: two lines in the feature list. Run `build.bat`.

## Not covered, on purpose

Other hotkey actions, toggle-to-talk, game controllers, per-server key profiles, a whisper history window, a sound on incoming whispers, per-contact whisper permission, whisper keys that only send while you speak, and whispering to a server you are connected to but not looking at.
