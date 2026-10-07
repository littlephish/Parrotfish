# PhishSpeak — Build Plan

A TeamSpeak 3 client in Rust with a Slint GUI. It logs in with real TS3 identities (P-256),
joins a server, shows the channel tree, chats, and does voice (Opus) both ways.

## Status (2026-10-06)

Working end to end against a real TeamSpeak 3.13.8 server: login, channel tree, channel
switching (incl. password channels), text chat, microphone capture → Opus → server, and
server → Opus → speakers, with and without voice encryption. 195 unit tests green.

The window is the compact tree layout in the Twilight reef palette (design:
`docs/superpowers/specs/2026-10-06-compact-window-design.md`): spacer channels are drawn as
dividers, servers can be bookmarked, several servers can be connected at once (you hear all
of them, the microphone goes to the one you are viewing), and every setting lives in a
separate tabbed settings window.

Talk keys are chosen by pressing them: any key, mouse button 3 to 5, or a combination of up to
four keys, several at once if wanted, with an optional delay before the microphone closes.
Whisper keys send your voice to chosen channels and people, or to a group of people in the
channels above, below or around yours, and never to your own channel; there is a reply key,
people whispering to you are marked, and incoming whispers can be switched off
(plan and decisions: `docs/superpowers/plans/2026-10-06-talk-and-whisper-keys.md` and its
`.ledger.md`).

Channels fold: an arrow beside every channel that has people or channels inside it. A setting
chooses how they start (all open, empty ones folded, all folded) and each channel you fold or
open yourself is remembered per server.

Echo cancelling (Settings, Microphone, off by default) takes the sound of your own speakers
out of your microphone, so people do not hear themselves when you listen through speakers.

Each bookmark can name the channel to join when you connect: pick it under Settings,
Bookmarks, or use "Start in <channel> next time" in the server menu.

Releases are built by GitHub Actions only when a `v*` tag is pushed (`.github/workflows/`,
`tools/package_release.py`, `installer/phishspeak.iss`); ordinary pushes run the tests only.

Run it:

```
cargo run --release -p ps-app                      # GUI
cargo run --release -p ps-app -- --connect <bookmark name or host[:port]> [--nickname NAME] [--channel NAME]
```

`--connect` can be given more than once; `--nickname` and `--channel` apply to the one before them.

Icons a server defines are shown: on channels, on people (channel group, server groups, their
own icon, at most four) and the server's own icon after its name. They are fetched once, two a
second at most, and kept in `%APPDATA%\PhishSpeak\cache\icons`. PNG and JPEG only.

A bookmark can connect when PhishSpeak starts and can keep a server password and a password
for its start channel. Passwords are stored encrypted for the Windows account, never as text.

Not done yet: ServerQuery browser (v1.1), private-chat tabs, permissions UI,
channel create/edit, file browser and avatars, GIF icons, SRV/TSDNS lookup (use `host:port`), legacy
Speex/CELT codecs (reported in the log, not decoded), pre-3.1 servers (`initivexpand`),
hotkeys for anything but talking, game controller buttons, noise suppression and automatic
gain.

Not yet verified by anyone: a conversation or a whisper with the **official** TS3 client
(everything so far is PhishSpeak ↔ real server ↔ PhishSpeak), and voice on a real
internet server (signing in to one has worked). Do that first before trusting it for daily use.
Also unverified: sound by ear, a talk key held on a real keyboard or mouse (the checks pressed
F13 to F24 by program), keys while a game running as administrator has the focus, two different
servers at once (the multi-server tests used two connections to one server), and the release
workflow and installer script, which have never run (there is no Inno Setup on this PC and
GitHub Actions cannot be run locally). Echo cancelling has been measured on simulated rooms
and on a sound device's own digital loopback, never in a real room with loudspeakers, never
by ear, and never with a microphone and speakers that are separate USB devices. The fix for
noise at the end of someone's speech is checked by tests on the decoded sound and by packet
order against the test server, not by ear and not with an official client talking.

## Goals

- **v1 — connect** ✅: import TS3 identities, connect over UDP (9987), complete the Init1 puzzle
  and the `clientinitiv` → `initivexpand2` → `clientek` → `clientinit` → `initserver` handshake,
  subscribe to all channels, sit connected in a channel.
- **v1.1 — server browser**: ServerQuery over TCP (10011) to list servers/status before connecting.
- **v2 — voice** ✅: Opus 48 kHz / 20 ms capture and playback, `Voice` packets, jitter buffer,
  loss concealment, voice activation / push-to-talk / continuous.
- **v2.1 — chat & channels** ✅ (basic): channel + server text chat, channel tree, joining channels.
  Permissions UI still open.

## Crates

| Crate | Responsibility | Status |
|---|---|---|
| `ps-identity` | INI parse, identity (de)obfuscation, DER, P-256, UID, hashcash level, sign/verify, generate/save | done, 14 tests |
| `ps-crypto` | EAX-AES128 (8-byte MAC), dummy key, per-packet key/nonce, license chain, Ed25519 shared secret, RSA puzzle | done, 16 tests |
| `ps-protocol` | Packet headers, command escape/parse/build, QuickLZ + fragmentation, receive windows/generations, Init1 payloads, voice and whisper payloads | done, 34 tests |
| `ps-client` | Connection actor thread: handshake, ack/resend, ping, command dispatch, channel/client/group book, voice and whispers in/out, events; `spacer` recognises spacer channels, `filetransfer` fetches icons over the server's file port | done, 20 tests + live tests |
| `ps-voice` | Opus codec, resampler, jitter buffer + mixer (talkers keyed by connection and client), VAD/PTT gate, lanes (which key is held decides where a frame goes), echo canceller (`echo.rs`), cpal device I/O (WASAPI) | done, 55 tests + live tests |
| `ps-app` | The windows. `session.rs` one connection (events, tree rows and folding, chat history), `app.rs` all sessions, the viewed one and where the microphone goes, `app/shortcuts.rs` choosing keys, the whisper key editor and the lane table, `hotkeys.rs` key combinations and what counts as held, `keywatch.rs` the thread that reads the keys, `whisper.rs` whisper keys and their file, `bookmarks.rs`, `settings.rs`, `platform.rs`, `ui/` theme, widgets, main and settings windows, `icons.rs` checks, shrinks and caches icons | done, 56 tests + live tests |
| `ps-serverquery` | Text protocol over TCP 10011 | not started |

Threads: UI thread (Slint, 33 ms timer drains client events) · `ps-client` actor + UDP reader ·
`ps-keys` (asks Windows every 5 ms which bound keys are down and tells the audio engine) ·
`ps-voice-tx` (mic → resample → gate → Opus → client) · `ps-voice-devices` (owns cpal streams,
reopens on device change) · cpal callbacks (capture into a ring buffer; playback pulls
jitter-buffer → decode → mix → resample).

## Protocol notes (verified against tsdeclarations `ts3protocol.md`, TSLib, tsclientlib, ts3j and a live 3.13.8 server)

Earlier notes in this file were wrong in places (4-byte header, "Init2/Init3", "ConnectRequest/
ConnectOk", "level 213"). What is actually on the wire:

### Identity
- `identity="<keyOffset>V<base64>"`; the number before `V` is the hashcash **key offset**, not the
  level. Level = number of leading zero bits (LSB-first per byte) of
  `SHA1(omega_base64 + decimal(keyOffset))`.
- Deobfuscation: base64 → first NUL at index ≥ 20 → `SHA1(data[20..20+nullIdx])` XOR'd into
  `data[0..20]`; then first 100 bytes XOR'd with the fixed ASCII key; result is base64 of the DER.
- DER (libtomcrypt): `SEQ { BIT STRING(7 unused, 0x80 priv / 0x00 pub), INT 32, INT X, INT Y [, INT k] }`.
  Integers must be **minimal** DER (leading zero bytes stripped) or the UID/omega is wrong.
- `UID = base64(SHA1(ascii(base64(pub_der))))`. Test vectors: the keys published by tsclientlib,
  in `ps-identity/tests/vectors.rs`.

### Packets
- C→S: `MAC(8) | PId u16 | CId u16 | PT u8 | data` (13-byte header); S→C: `MAC(8) | PId u16 | PT u8 | data` (11).
  `PT` low nibble = type (0 Voice, 1 VoiceWhisper, 2 Command, 3 CommandLow, 4 Ping, 5 Pong, 6 Ack,
  7 AckLow, 8 Init1); flags 0x80 Unencrypted, 0x40 Compressed, 0x20 Newprotocol, 0x10 Fragmented.
  Max 500 bytes per datagram. The generation id is **not** transmitted; it is counted per type
  each time the 16-bit packet id wraps.
- Encryption: AES-128-EAX, 8-byte tag in `MAC`, header after the MAC is the associated data.
  `key|nonce = SHA256(0x30 (S→C) / 0x31 (C→S) | type | generation u32 BE | SharedIV[64])`, then
  `key[0..2] ^= PId`. Before the session keys exist: key `c:\windows\syste`, nonce `m\firewall32.cpl`.
- Unencrypted packets (Ping, Pong, plain voice) carry `SharedMac = SHA1(SharedIV)[0..8]` as MAC;
  Init1 carries `TS3INIT1`.
- Commands > 487 bytes: QuickLZ level 1 if it helps, then split; `Fragmented` on first and last
  part, `Compressed` only on the first.

### Handshake
- Init1 (packet id 101, flags 0x88): C `[version u32][0x00][unix time u32][random 4][8 zero]` →
  S `[0x01][16B][4B]` → C `[version][0x02][16B][4B]` → S `[0x03][x 64][n 64][level u32][100B]` →
  C `[version][0x04][x][n][level][100B][y 64][clientinitiv text]`, `y = x^(2^level) mod n`.
  Step `0x7F` from the server means "start over".
- `clientinitiv alpha=<b64 10 random> omega=<b64 pub DER> ot=1 ip` rides in Init step 4 and counts
  as Command packet id 0, so the first real command (`clientek`) has id 1.
- Server: `initivexpand2 l=<license> beta=<54B> omega=<server pub> ot=1 proof=<ECDSA(l)>`
  (Command id 0, dummy key). Verify `proof` with the server's `omega`.
- License: `[version 1]` then blocks `[0x00][pubkey 32][type][notBefore u32][notAfter u32][content]`
  (types 0 intermediate, 1 website, 2 server, 3 code, 8 TS5 server, 32 ephemeral).
  `key = pubkey * clamp(SHA512(block[1..])[0..32]) + parent`, starting from the fixed root key.
- Client makes an Ed25519 keypair; `SharedIV = SHA512(compress(priv * derivedKey))`, bytes 0..10 XOR
  alpha, 10..64 XOR beta. Send `clientek ek=<b64 pub> proof=<b64 ECDSA_P256_SHA256(ek || beta)>`
  **still with the dummy key**; everything after uses the session keys. The ack for `clientek` may
  arrive under either key.
- `clientinit client_nickname client_version client_platform client_input_hardware
  client_output_hardware client_default_channel client_default_channel_password
  client_server_password client_meta_data client_version_sign client_key_offset
  client_nickname_phonetic client_default_token hwid` (empty values are sent as bare keys;
  passwords are `base64(sha1(pw))`). Version triple used: `3.?.? [Build: 5680278000]` / `Windows` /
  the matching sign from tsdeclarations `Versions.csv`.
- Server: `initserver … aclid=<client id>`, `channellist…`, `channellistfinished` (→ we send
  `channelsubscribeall`), `notifycliententerview…`. Refusals arrive as `error id=… msg=…`;
  `0x0207` with `extra_msg=<n>` means "identity security level n required".

### Voice
- C→S `[voice id u16][codec u8][opus]`, S→C `[voice id u16][client id u16][codec u8][opus]`;
  codec 4 Opus Voice (mono), 5 Opus Music (stereo); 48 kHz, 20 ms frames. Empty opus data ends a
  talk spurt (tsclientlib also counts one byte as empty, and so does PhishSpeak; a one-byte
  Opus packet carries no sound). Voice id = the Voice packet id.
- The server can hand on two packets sent back to back in the other order (live, 3.13.8: an
  end packet sent right behind the last voice packet arrived first in three of four tries). So
  PhishSpeak sends its end packet one frame later, and a receiver must treat a voice packet
  that is older than the end packet as the tail of that speech, not as new speech.
- Encrypt voice when `virtualserver_codec_encryption_mode` is 2, or 0 and the channel has
  `channel_codec_is_unencrypted=0`; otherwise send with the Unencrypted flag + SharedMac.
- Multi-item notifications (`a=1 b=2|b=3`) inherit missing keys from the first item.

### Start channel
- `client_default_channel` in `clientinit` takes the channel's path by names, joined with `/`,
  with `\/` for a slash inside a name (`Deep Rock/Radio`). A bare sub-channel name, a path that
  does not exist and the `/<id>` form all leave you in the server's default channel, without an
  error. So does a locked channel when no password is sent (live, 3.13.8).

### Icons and file transfer
- Ids arrive in `channel_icon_id`, `client_icon_id`, `virtualserver_icon_id` and as `iconid`
  in the group lists. The same id is written three ways (`2154984321`, `-2139982975`,
  `18446744071569568641`); all fold to one 32-bit number. 100, 200, 300, 500 and 600 are the
  standard group icons and have no file. An id is only a file name, not a checksum to verify.
- Who is in which group: `client_servergroups` (comma list) and `client_channel_group_id` at
  `notifycliententerview`, then `notifyservergroupclientadded` / `...deleted` (`sgid`, `clid`)
  and `notifyclientchannelgroupchanged` (`cgid`, `cid`, `clid`). The last one is also sent by
  the server itself right after `notifyclientmoved`, so the channel group follows people.
- Download: `ftinitdownload clientftfid=N name=/icon_<id> cid=0 cpw seekpos=0 proto=1`, answered
  by `notifystartdownload clientftfid serverftfid ftkey port size`; open TCP to the voice
  server's address on that port, send the key, read `size` bytes. A missing file is answered
  with `notifystatusfiletransfer status=2054`; `ftstop serverftfid=N delete=0` cancels.
- Flood protection (defaults 5 / 150 / 250): 110 `ftinitdownload` sent at once were not
  blocked; of a burst of nickname changes the ninth was. A blocked command gets `error id=524 ... extra_msg=retry
  in Nms` with its `return_code`.

### Whisper
- To a list, packet type VoiceWhisper, `Newprotocol` flag clear:
  `[voice id u16][codec u8][N u8][M u8][N channel ids, u64 each][M client ids, u16 each][opus]`.
- To a group, packet type VoiceWhisper, `Newprotocol` flag set:
  `[voice id u16][codec u8][who u8][where u8][id u64][opus]`. Who: 0 server group, 1 channel
  group, 2 channel commanders, 3 everyone. Where: 0 all channels, 1 current, 2 parent, 3 all
  parents, 4 channel family, 5 complete family, 6 subchannels. The id is the group id for who 0
  and 1 and is ignored otherwise.
- Received whispers look like voice (`[voice id][client id][codec][opus]`) and are told apart by
  the packet type. An empty opus part ends the whisper. Encryption follows the voice rule.
- Who hears what (live, 3.13.8; Deep Rock contains Radio and Drift, Radio contains Booth):

  | Where | Sender in Radio | Sender in Booth | Sender in Deep Rock |
  |---|---|---|---|
  | all channels | everyone on the server | | |
  | current | Radio | Booth | Deep Rock |
  | parent | Deep Rock | Radio | nobody |
  | all parents | Deep Rock | Radio, Deep Rock | nobody |
  | channel family | Radio, Booth | Booth | Deep Rock, Radio, Booth, Drift |
  | complete family | Deep Rock, Radio, Booth, Drift | the same four | the same four |
  | subchannels | Booth | nobody | Radio, Drift (not Booth) |

- A list reaches the people in the listed channels plus the listed people, and nobody else.
  Unknown ids are skipped. The codec byte is passed through untouched.
- When nobody hears a whisper the server answers `error id=1804 (0x070c) msg=no whisper targets
  found`, once per burst. That covers an empty list, a group or place that matches nobody, and a
  listener who requires more whisper power than the sender has; the cases cannot be told apart.
- `clientupdate client_is_channel_commander=1` needs `b_client_use_channel_commander`; without
  it the server answers `0x0a08`.
- After login the server sends `notifyservergrouplist` and `notifychannelgrouplist` unasked:
  group ids, names, `type` (1 is a regular group) and `sortid`.

## Verification done

- Unit tests with third-party vectors: tsclientlib's license derivation, shared IV, key/nonce,
  dummy-key packet, a captured `clientinit` packet decrypt, a real server's license signature,
  UID and security-level vectors; identity export round trip.
- Live against TeamSpeak 3.13.8 (Linux, in WSL): handshake with a generated identity and with
  one exported from the TeamSpeak client; channel/server password; locked channel (right and wrong password); automatic
  security-level upgrade when the server demands more; chat incl. fragmented + compressed
  1000-character messages; voice round trip between two clients (Opus Voice and Opus Music,
  encryption off and forced on) byte-exact; 70 000 encrypted voice packets across the 16-bit
  packet-id wrap; handshake + chat with 25 % simulated loss each way.
- Compact window, live against the same server, driven with real clicks and keystrokes and
  checked in software-rendered screenshots: every spacer form (centred, left, right, repeated,
  dashed, dotted, gap) and a channel named `[cspacer` that stays an ordinary, joinable channel;
  joining a locked channel through the password prompt; channel and server chat incl. a refusal
  shown as a readable line; the six settings tabs and their controls; creating an identity and a
  bookmark; a bookmark surviving a restart and connecting with one click; a failed connect
  reopening the dialog with the reason under the address; the identity being strengthened
  automatically (level 10 → 23) and the connection retried; the window size being remembered.
- Two connections at once with two identities in different channels, with a headless listener
  in each channel (`channeltest`): voice arrived only in the viewed connection's channel at
  50 packets/s; switching the view delivered an end-of-talk packet to the channel left behind
  and the first packet to the other one 12–14 ms later; disconnecting the viewed connection
  moved the view and the microphone to the remaining one; talkers in both channels were shown
  at the same time (in the tree and on the other connection's tile); muting the microphone or
  the sound stops the stream with an end-of-talk packet.
- Audio devices on this PC: capture runs at 48 kHz from the Arctis and webcam mics; a −48 dBFS
  test tone pushed through the playback path was read back from the headphone endpoint via
  WASAPI loopback at −48.0 dBFS.
- Talk keys, live, with keys pressed by `tools/hold_keys.py` (F13 to F24, which no keyboard has)
  and a listener in the channel: an old `ptt_key=` setting came up as its key and was rewritten
  as `talk_key=`; a 2 s press gave a first packet 14 ms after the key went down, 100 packets and
  an end-of-talk packet; with a 0.3 s release delay 115 packets; a two-key combination sent only
  while both keys were down; a key chosen in the settings window by pressing it; choosing a key
  and closing the window without pressing one left the talk key working.
- Whisper keys, live, six listeners spread over the channels, keys made in the editor with
  clicks: "everyone, the channel above mine" from Radio reached only Deep Rock and "the channels
  right below mine" only Booth (100 whisper packets and one end marker each); a list of one
  channel and one person reached exactly those; a key that belongs to another server and a key
  whose channel is gone reached nobody, the dock said why, and the listener in the app's own
  channel heard nothing in any whisper check; pressing a whisper key in the middle of a held talk
  key gave the channel 50 voice packets, an end-of-talk, 50 more and an end-of-talk, and the
  parent 50 whisper packets and an end-of-whisper; a whisper to the channel above from a top-level
  channel showed "Nobody is there to hear that whisper" and added no chat line; a whisper from a
  music-quality channel with thirty channels in the list arrived as codec 4 while talk in that
  channel stayed codec 5; a whisper to the app marked the sender and added one chat line, the
  reply key reached that sender, and with incoming whispers off nothing was marked or played;
  becoming channel commander showed the mark, a whisper to all commanders reached only the app,
  and without the permission the server's refusal was shown; muting or switching servers in the
  middle of a whisper sent the end marker at once and nothing after it; with two connections the
  whisper went out on the viewed one. `whispertest` reproduces the who-hears-what table (12 of 12).
- Start channel, live: a bookmark with a path lands in that channel at login (also with a slash
  in the name); after the channel was renamed the remembered id is used and the client moves
  there right after connecting; a channel that no longer exists leaves you in the default
  channel; a locked start channel opens the password prompt on arrival and joins with the right
  password; setting and clearing it from the server menu and picking it in the bookmark editor
  both end up in `bookmarks.ini` and are used at the next start.
- Echo cancelling. On simulated rooms (a speech-like far end, a room echo of 30 ms after a
  delay of 15 to 500 ms, my own voice on top, 14 tests): 47 dB of echo removed once settled, 38 dB
  of it by the adaptive filter alone and 26 dB already in the second second; my voice is
  untouched when nothing plays or when what plays does not reach the microphone (headset);
  talking over the echo changes my level by 0.5 dB; a changed room is relearned (39 dB again
  four seconds later);
  a reference that arrives late or early, or stops and comes back, does not lose what was
  learned; 60 ppm of clock difference between the two devices is measured as 59 ppm and followed
  (34 dB); an echo half a second late is found and lined up (48 dB); 3 to 4 % of one processor
  while something plays, 0.3 % otherwise. On real device timing, through the whole engine
  (`echotest --loopback` on a silent virtual output, the device's own signal as the
  microphone): the sound came back 5 to 16 ms after it was played and 52 dB of it was removed
  after 16 seconds (31 dB within the first 8). The pair "Speakers (Steam Streaming Microphone)"
  to "Microphone (Steam Streaming Microphone)" is not usable as a test: what comes back does not
  line up with what was played at any one delay, and nothing is removed there.
- Icons, live against the test server: channel, group, personal and server icons shown; a
  64 x 64 picture shrunk; JPEG shown; a GIF, a 300 x 300 picture, an HTML file and a 600 KiB
  file refused without any sign in the window (the last one stopped before any byte was
  fetched); icons following changes made while connected (channel icon added and removed,
  server icon replaced, joining and leaving Server Admin, channel admin in one channel only
  and moving in and out of it, a personal icon); after a restart the cached files were used
  and left untouched; with the file port unreachable three tries are made and the rest given
  up for ten minutes while the connection carries on; 40 icons at default flood protection
  with no complaint; 500 voice packets arrived complete while 40 icons were fetched on the
  same connection. On `ts.busaesi.space` (3.13.7) the server icon and the group icons of the
  people there were fetched and drawn.
- Bookmarks, live: a channel password typed in the editor is written as `dpapi:` hex and the
  word itself is not in the file; with "connect when PhishSpeak starts" the program connected
  by itself and went straight into the locked channel; a server password sealed by Windows'
  own tools with the same purpose text was accepted; a wrong saved password and an address
  that does not answer gave a notice, not the connect dialog.
- End of speech: seven new tests on the decoded sound (no step at the end with the end packet
  in time, late, one byte long or missing; nothing audible 10 ms after a marked end and 60 ms
  after an unmarked one; a late last packet still ends cleanly; old packets do not start a new
  stream; talking again at once plays no filler). Before the change the same tests showed up
  to 140 ms of made-up sound after the last packet and a step seven times the usual size.
- Folding, on screen: a channel folds and opens from its arrow, an empty branch starts folded,
  the three "Channels start" choices change the tree at once, a folded channel shows how many
  people are inside and tints its icon while one of them talks, joining a folded channel opens
  the way to it, and a fold made by hand was still there after a restart.

Dev tools (examples): `cargo run -p ps-client --example probe -- <host> [--identity file] [--say TEXT]
[--join CID] [--loss 0.2] [--auto-level] [--log]`, `cargo run -p ps-voice --example voicetest --
<host> [--music] [--listen] [--burst 70000] [--loss 0.2]`, `cargo run -p ps-voice --example
devicetest -- [--input NAME] [--tone]`, `cargo run -p ps-voice --example channeltest -- <host>
[--nick NAME] [--join CID] [--seconds N] [--talk SECONDS] [--whisper client:ID|channel:ID|commanders|everyone]
[--commander]` (sits in one channel and reports every voice, whisper and end packet it hears,
and the sound formats; with `--talk` it also sends a tone, as a whisper with `--whisper`),
`cargo run -p ps-voice --example echotest -- [--output NAME] [--input NAME | --loopback]
[--seconds N] [--level DB]` (plays a speech-like test sound and reports how loudly the input
hears it with echo cancelling off and on, when the sound came back and the clock difference;
it is audible unless the output is a virtual device),
`cargo run -p ps-client --example whispertest -- <host> --booth CID --drift CID` (re-runs the
who-hears-what table and fails if a server behaves differently).
`PHISHSPEAK_TRACE=1` makes the GUI show every command in the chat drawer.
`probe` also takes `--icon ID` (repeatable), `--all-icons`, `--save DIR`, `--ft-port N` and
`--voice` (how each talker's stream ends: packet sizes and timing, no sound).

Scripts in `tools/`: `hold_keys.py F13..F24[+F13..F24] <seconds>` presses keys no keyboard has,
for checking talk and whisper keys; `seed_whisper_tree.py --password <query password>` (run
where the test server's query port is reachable) makes the Booth and Drift channels and lets
guests be channel commanders; `seed_test_icons.py --password <query password>` puts good and
deliberately bad icons on the test server (`--count N`, `--upload FILE`, `--assign WHAT=ID`,
`--clear`); `package_release.py [--tag vX.Y.Z] [--skip-installer]` builds the
release program with the C runtime linked in and writes the zip, the installer (needs Inno
Setup 6) and their checksums to `dist/`.

A local test server: official `teamspeak3-server_linux_amd64` in WSL
(`./ts3server license_accepted=1`, which accepts TeamSpeak's server license), reachable from
Windows at the WSL IP (`hostname -I`). Many quick reconnects trip its anti-flood ban; raise
`virtualserver_antiflood_points_needed_ip_block` via ServerQuery while testing.

## Known issues / decisions

- `unsafe-libopus 0.2.0` (libopus 1.3.1 transpiled to Rust, chosen because no CMake/C toolchain
  step is needed): its **SILK packet-loss concealment outputs full-scale noise**. PhishSpeak
  therefore never calls the decoder's PLC/FEC for SILK/hybrid streams and conceals lost frames
  itself (reverse/forward repeat of the last frame with a fade); CELT streams use the decoder's
  PLC. Revisit if the crate is fixed or when switching to C libopus (needs CMake).
- Windows capture streams only accept the device's native format, so both directions go through
  our own windowed-sinc resampler.
- Settings live in `%APPDATA%\PhishSpeak\settings.ini`, bookmarks in `bookmarks.ini` next to
  it, identities created in the app in `%APPDATA%\PhishSpeak\identities`. Importing an identity
  remembers where the file is; it is never copied and never modified, and an improved key offset
  is cached in settings. A password typed in the connect dialog or a channel prompt is used for
  one attempt and never written to disk. A password typed into a bookmark is kept: sealed with
  Windows' data protection for the signed-in account (`dpapi:` and hex in `bookmarks.ini`), so
  the file is useless on another PC or account, where the bookmark simply has no password. A
  line with a readable password is ignored. On systems without that protection nothing is
  saved.
- Every bookmark marked to connect at start is connected, and the first of them in the list is
  the one shown. If one fails, a notice says so; the connect dialog is not opened for it.
- One bookmark per server address. A second bookmark for the same server needs a different
  spelling of the address (for example with the port).
- Several servers: every connection is heard; the microphone goes to the viewed one only. Mute
  applies to all. When the view changes mid-sentence the server left behind gets an end-of-talk
  packet, and on mute the end-of-talk packet is sent before the server is told we are muted
  (it drops voice from muted clients, so the other order leaves listeners waiting for a timeout).
- Amber (Lure) is reserved: someone is talking, where you are (viewed server, active tab,
  keyboard focus, your own name in chat), and the one main button of a dialog. Error and
  secondary text use lighter tints on raised and highlighted surfaces to keep 4.5:1 contrast.
- The minimum window size (340 × 520) is declared in the UI; it has not been checked by
  dragging the window border.
- Keys are read by asking Windows 200 times a second whether each bound key is down
  (`GetAsyncKeyState`); no keyboard or mouse hook is installed. That works while another program
  has the focus. Not seen: keys of a game that runs as administrator (unless PhishSpeak does
  too), and buttons on controllers, joysticks and pedals. Left and right mouse buttons cannot be
  bound; Esc alone cancels choosing a key. Old `ptt_key=<number>` settings are read once and
  rewritten as `talk_key=<key codes joined with +>`.
- A whisper key opens the microphone by itself whatever "Send my voice" says, acts on the server
  being viewed, and never falls back to the channel: if its targets are gone, offline or on
  another server, the frames are dropped and the dock says why. Whispers are always Opus Voice,
  and a frame is encoded into the room the target list leaves in the packet (30 channels and 60
  people at most, 122 bytes left). Every audience gets its end marker when the stream to it
  stops: key released, another key pressed, mute, or a change of viewed server.
- Whisper keys live in `%APPDATA%\PhishSpeak\whisper.ini`. People are stored by TeamSpeak UID
  and last known name, channels by id and name, groups by id, name and the server's UID. A key
  that names channels, people or a group only works on the server it was made for.
- The reply target is the last person who whispered to you on the viewed server, checked by UID
  so a client id that was handed to someone else is not used. It does not change while the reply
  key is held.
- Channel folding: the setting `fold_mode` (0 all open, 1 empty ones folded, 2 all folded) gives
  the starting state; "empty" means a channel with channels inside it and nobody in any of them.
  The way to your own channel is open when you arrive. What you fold or open by hand overrides
  the setting, is stored per server UID as `folds.<uid>=<channel id>:<0|1>,…` in `settings.ini`
  (512 channels a server, 64 servers), and is forgotten when the setting is changed.
- Echo cancelling is PhishSpeak's own code in `ps-voice/src/echo.rs`, no new crate. The
  reference is what is written to the output device (one channel, after volume), sent to the
  transmit thread through a ring buffer. A block frequency-domain adaptive filter (blocks of
  256 samples, 64 partitions, 341 ms) learns the path from the speakers to the microphone; a
  second copy of the filter is only updated when the adapting one has proved better, so a wrong
  turn during double talk does not reach your voice. What the filter leaves is turned down by a
  suppressor that acts only on the part of the output that still moves with the echo estimate,
  and never below the room's own noise. Three helpers keep the filter lined up: far and
  microphone blocks are paired by count and a block is re-used or skipped (with the learned
  filter shifted to match) when one side runs late or early; a delay finder compares loudness
  over time and holds the reference back when the echo arrives more than about 130 ms late (up
  to roughly 1.1 s); and the slow turning of the learned filter's phase gives the clock
  difference between the two devices, which the reference is resampled to remove.
  Costs and limits: your voice is delayed by 10.7 ms while it is on; the reference is mono, so
  wide stereo music leaves more behind than speech; the echo must arrive at least a few
  milliseconds after it is played (ordinary Windows devices are 30 ms and more); a device pair
  whose delay keeps jumping cannot be cancelled; the level meter and "When I speak" work on the
  cleaned signal, so your speakers no longer open the microphone. Off by default.
- The audio engine can open an output device as its microphone (the device's loopback). The
  app never offers that; `echotest --loopback` uses it.
- A bookmark's start channel is stored as the channel's path and its id (`channel=`,
  `channel_id=` in `bookmarks.ini`). The path is sent at login. If you land somewhere else, the
  channel is looked up by path and then by id and joined, with the bookmark's channel password
  if it has one, else through the password prompt. `--channel` on the command line overrides
  it for that start.
- Icons: an icon is asked for only when a row that needs it is shown, one at a time, at most
  two a second. The download goes only to the voice server's own address. A file is checked
  before it is decoded: at most 512 KiB, PNG or JPEG by its first bytes, at most 256 x 256;
  SVG from a server is never drawn. Pictures larger than 32 x 32 are averaged down once.
  Files are kept per server (folder named by the server id in hex) as `icon_<id>.png|jpg`, at
  most 500 per server, and fetched again after 7 days while the old one stays on screen. A
  failed download is retried after ten minutes, a refused file not again in that run. Slint
  finds a file's format from its name and caches decoded files by path and whole second,
  hence the extensions. On a person the order is channel group, server groups in the server's
  order, own icon. The five standard group icons are drawn for PhishSpeak.
- End of speech: the last frame is faded over its final 8 ms when the end packet is already
  there, otherwise 8 ms of the last sound are mirrored and faded. Without an end packet the
  filler fades within 60 ms instead of 120. A voice packet older than the end packet belongs
  to that speech; one that arrives after the speech has ended is dropped.
- Release builds: the workflow `Release` runs only for a pushed tag `v<version>`, checks that
  the tag matches the version in `Cargo.toml`, runs the tests, builds with the C runtime linked
  in (`-C target-feature=+crt-static`, so no Visual C++ runtime has to be installed), and
  publishes `PhishSpeak-<version>-setup.exe`, a zip of the program and `SHA256SUMS.txt` as a
  GitHub release. The installer is per user (no administrator prompt) and leaves
  `%APPDATA%\PhishSpeak` alone when uninstalling. The files are not code-signed, so Windows
  SmartScreen will warn. Before publishing a build, choose the Slint licence it is distributed
  under (see README).

## Milestones

1. ✅ `ps-identity`: decode, verify, UID.
2. ✅ `ps-app` Slint shell.
3. ✅ UDP framing + Init1 against a local TS3 server.
4. ✅ Crypto handshake + `clientinit` → `initserver`; client appears in a channel.
5. ServerQuery browser (TCP 10011) feeding the server field.
6. ✅ Voice: Opus encode/decode + device I/O, send/receive `Voice`.
7. ✅ Text chat + channel tree (basic). Private chats, pokes UI, permissions: open.
8. ✅ Identity management (create / import), settings window, bookmarks, several servers at once,
   spacer channels, compact window.
9. ✅ The icons a server defines for channels, people, groups and itself
   (`docs/superpowers/plans/2026-10-06-custom-icons.md`, with its `.ledger.md`). GIF icons are
   left out until the `gif` crate is wanted as a direct dependency.
10. ✅ Talk keys of your choice and TeamSpeak-style whisper keys
    (`docs/superpowers/plans/2026-10-06-talk-and-whisper-keys.md`; what was decided on the way is
    in the `.ledger.md` beside it).
11. ✅ Folding channels with a starting-state setting and remembered choices; release workflow
    and installer script (written, never run).
12. ✅ A start channel per bookmark; echo cancelling for the microphone.
13. ✅ Bookmarks that connect at start and keep passwords; clean ends of speech.
14. Next: test against the official client and a public server; try echo cancelling in a real
   room; SRV/TSDNS resolution; noise suppression / AGC; per-user volume in the UI (the mixer
   already supports it).

## References

- `ReSpeak/tsdeclarations` — `ts3protocol.md` (the wire spec), `Messages.toml`, `Errors.csv`, `Versions.csv`.
- `ReSpeak/tsclientlib` — `tsproto` (Rust): packet codec, license, test vectors.
- `Splamy/TS3AudioBot` — `TSLib/Full/`: `TsFullClient.cs`, `PacketHandler.cs`, `TsCrypt.cs`, `License.cs`.
- `Manevolent/ts3j` — Java client, same handshake.
- Local TS3 client at `C:\Program Files\TeamSpeak 3 Client` for behaviour comparison.
- Rust crates: `slint 1.18`, `p256 0.13`, `sha1`/`sha2 0.10`, `aes 0.8` + `eax 0.5`,
  `curve25519-dalek 4`, `num-bigint 0.4`, `quicklz 0.3`, `cpal 0.18`, `unsafe-libopus 0.2`, `rtrb 0.3`.

## Slint 1.18 API notes (verified against registry source)

Build/compile:
- External `.slint` files compile via `slint_build::compile("ui/main.slint")` in `build.rs`
  + `slint::include_modules!();` in `main.rs`. The `slint!` macro is inline-only and does not
  expand `include!(concat!(env!(...)))`.
- Components land at the `include_modules!` site (crate root): `PhishSpeakApp::new()`.
- Event loop is the free function `slint::run_event_loop()` — there is no `EventLoop` type.

Rust ↔ .slint API:
- **Property visibility matters**: a plain `property <t> x;` is *private* — not exposed to
  Rust at all. Use `in property` (Rust sets, UI reads), `out property` (UI sets, Rust reads),
  `in-out property` (both). `<=>` two-way binding only works with `in-out`; assigning inside
  the component (e.g. a TouchArea) also requires `in-out`.
- **Rust accessors are prefixed** (`accessor_names.rs`): property `x-y` → `get_x_y()` /
  `set_x_y(v)`; callback `name` → `on_name(move || ...)` / `invoke_name()`; `public function f(a)`
  → `invoke_f(a)`.
- Components do not implement `Clone`; use `app.clone_strong()` or `app.as_weak()`.
- Model row types: declare `struct Row { nickname: string, ... }` **in the .slint file**
  (`export struct` to use it from Rust); the Rust struct is generated at the `include_modules!`
  site (`#[derive(Default, PartialEq, Debug, Clone)]`, pub fields, `string` → `SharedString`).
- The `slint::Model` derive macro no longer exists. `VecModel<T>` is not `Clone`; share it as
  `Rc<VecModel<T>>`, hand the UI `ModelRc::from(rc.clone())`, replace rows with `set_vec`,
  update one row with `set_row_data`.
- Background threads never touch the UI: they send over `mpsc` and a `slint::Timer` on the UI
  thread drains the channel.

`.slint` syntax (1.18, verified in compiler source):
- Repeaters: `for x in model: Elem { ... }` — colon + repeated element, no `repeater`
  keyword, no bare block. No implicit index; use `for x [idx] in model:` for a custom name.
- `row`, `col`, `rowspan`, `colspan` are reserved layout properties on every element — do not
  name your own property `row`.
- Equality is `==` (not `===`); multi-statement handlers need braces:
  `clicked => { root.foo = 1; }`.
- All lengths need units (`880px`, `12px`).
- Window sizing: cannot set both `width` and `min-width`; use
  `preferred-width`/`preferred-height` for the initial size + `min-width`/`min-height`.
- `Rectangle`: `border-radius` (not `radius`), `min-width`/`min-height`
  (`minimum-*` is deprecated).
- `Text`: wrapping is `wrap: word-wrap;` + an explicit `width`; `font-weight: 700` is an int
  (bare `bold` is invalid).
- std-widgets (fluent): there is no `TextBox` — use `TextEdit` (multi-line; `text` is
  in-out, `read-only: true`) or `LineEdit` (single-line; `input-type: InputType.password;`,
  no `echo-mode`, no `fixed-width` — use `width` + `horizontal-stretch`).
- Scroll state is `content-x/y/width/height` + `visible-width/height` (`viewport-*` is deprecated).
  To keep a read-only `TextEdit` scrolled to the end, move its cursor there:
  `edit.set-selection-offsets(len, len)` (byte length). Setting `content-y` directly is undone by
  the cursor-follow logic.
- `Button` has an `out property <bool> pressed`; `changed pressed => { ... }` gives hold-to-talk.
- `Palette.color-scheme = ColorScheme.dark;` in `init => { }` forces the dark fluent theme.
- Layouts: built-in elements are `VerticalLayout`/`HorizontalLayout`/`GridLayout`
  (PascalCase); std-widgets `VerticalBox`/`HorizontalBox`/`GridBox` inherit them and add
  default spacing/padding. A fixed-size child is centred by wrapping it in
  `VerticalLayout { alignment: center; }`.
- Screenshots for checking layout: run with `SLINT_BACKEND=winit-software`; the default GPU
  renderer comes out blank in `PrintWindow` captures.

Learned while building the compact window:
- A second window is just another exported `Window` component: `SettingsWindow::new()`, then
  `show()` / `hide()`. The event loop only ends when every window is hidden, so the main
  window's `on_close_requested` has to hide the others.
- Key presses reach a `FocusScope` only while something inside it has focus. For Escape to work
  everywhere: wrap the window content in one `FocusScope`, give the window
  `forward-focus: <that scope>`, and call `scope.focus()` from `changed` handlers when an overlay
  closes. Unhandled keys bubble from a focused `TextInput` up to it.
- `init => { field.focus(); }` on the root element of an `if` block focuses a field when the
  overlay appears.
- An overlay sheet can size itself with `height: self.preferred-height;` when its content is a layout.
- A child whose `height` depends on `root.height` inside the root layout is a binding loop;
  share space with `vertical-stretch` instead.
- `overflow: clip` on a `Text` does not cut off a string wider than its box in the software
  renderer; put the text in a `Rectangle { clip: true; }`.
- `PopupWindow` (`popup.show()` / `popup.close()`) is enough for a themed dropdown; `@image-url`
  needs literal paths, so icons live in a global (`Icons.mic`), tinted with `colorize`.
- Window size: `window().set_size(LogicalSize)` before `run()`, and
  `window().size().to_logical(window().scale_factor())` in the close handler.
- A test script can drive the window by posting `WM_MOUSEMOVE`, `WM_LBUTTONDOWN/UP` and
  `WM_KEYDOWN/UP` to it (lower-case letters, digits and unshifted punctuation only), which
  works while the window is behind others.

Learned while building the Shortcuts tab and folding:
- `FocusScope` has `capture-key-pressed(event) -> EventResult`, called from the window down to
  the focused element before `key-pressed`. Returning `accept` there swallows every key, which
  is how nothing in the window reacts while a key is being chosen.
- Popups are drawn inside the window and are cut off at its edge. Every element has
  `absolute-position`; with the window height kept in a global (set from the window's `init`
  and `changed height`) a dropdown can open upward when there is no room below.
- An element inside an `if` can be named (`if cond: name := Elem { }`) and used by its siblings
  inside that `if`; it cannot be reached from outside, so a part that others read (the fold
  arrow's hover state) is always created and made inert instead.
- A child `TouchArea` above a parent's takes the click, so an arrow inside a row can be clicked
  without the row's double-click firing; the row loses its hover while the pointer is on the
  child, so the row asks the child too.
- A `Text` with `wrap: word-wrap`, `overflow: elide` and a fixed `height` shows as many lines as
  fit and elides the last one.
- Changing rows with `VecModel::insert` / `remove` / `set_row_data` instead of `set_vec` keeps a
  `ListView` where it was scrolled.

## Conventions

- No comments unless asked; follow existing code style per crate.
- No real identities, UIDs or machine paths in tests, documents or scripts. Use the keys
  tsclientlib publishes, or generate one in the test.
- Test/verification scripts in Python (not PowerShell).
- Each protocol struct gets a round-trip test with captured or TSLib-derived vectors.
