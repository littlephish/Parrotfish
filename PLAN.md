# PhishSpeak — Build Plan

A TeamSpeak 3 client in Rust with a Slint GUI. It logs in with real TS3 identities (P-256),
joins a server, shows the channel tree, chats, and does voice (Opus) both ways.

## Status (2026-10-07)

Working end to end against a real TeamSpeak 3.13.8 server: login, channel tree, channel
switching (incl. password channels), text chat, microphone capture → Opus → server, and
server → Opus → speakers, with and without voice encryption. 331 unit tests green.

The window is the compact tree layout in the Twilight reef palette (design:
`docs/superpowers/specs/2026-10-06-compact-window-design.md`): spacer channels are drawn as
dividers, servers can be bookmarked, several servers can be connected at once (you hear all
of them, the microphone goes to the one you are viewing and the others are told it is off),
and every setting lives in a separate tabbed settings window.

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
second at most, and kept in `%APPDATA%\PhishSpeak\cache\icons`. PNG and JPEG, and the first
frame of a GIF.

A bookmark can connect when PhishSpeak starts and can keep a server password and a password
for its start channel. Passwords are stored encrypted for the Windows account, never as text.

Added on 2026-10-07:

- An address without a port is looked up the way the TeamSpeak client does it: an SRV record
  (`_ts3._udp`), then TSDNS, then the plain name.
- Clicking a person opens a panel: private messages, poke, a volume slider and a mute for that
  person alone (remembered by their UID), and what the server tells about them. On your own row
  the panel sets you away.
- A lost connection is retried by itself after 2, 4, 8 and 15 seconds and then every 30, and
  you are put back in the channel you were in.
- Privilege keys can be used from the server menu. In a moderated channel you can ask to talk.
  People who record, and people who asked to talk, are marked.
- Right-clicking a channel shows its topic and description. A server's host message is shown
  when you sign in.
- Event sounds (connected, connection lost, someone joins, leaves or is moved, a message, a
  poke, microphone and sound muted and unmuted) with their own volume.
- For the microphone: steady background noise can be taken out and the level can be kept even.
  Both are off by default.
- Keys that mute the microphone and the sound, chosen like talk keys.
- A small separate window that lists who is speaking, and for a few seconds who just spoke
  (Settings, Channels, or the server menu). You move and size it, then lock it: locked, clicks
  pass through it and it is invisible until someone speaks. It can stay above other windows
  while the main window does not, and can be made see-through.
- A window dragged to a display with another scale keeps its size and its limits.
- Speex, the voice format of old TeamSpeak channels, is played in all its three kinds.
  PhishSpeak itself always talks in Opus. CELT, the other old format, is not played.
- `ts3server://` links: a switch under Settings, Bookmarks makes PhishSpeak the program that
  opens them. A link never connects by itself: it opens the connect dialog filled in, with a
  line saying what else the link carries. Starting PhishSpeak while it is already running
  hands the link (or a `--connect`) to the running one.
- With several servers connected, the ones that do not have your microphone are told it is
  switched off, as the TeamSpeak client does for its other server tabs. People there see that on
  your name, and the server itself passes on none of your voice. The list of connected servers
  marks which one has the microphone.
- Versions 0.1.0, 0.2.0, 0.3.0 and 0.3.1 were built and published by the release workflow.

Planned, not built: reading keys through Windows' Raw Input as a switch in settings
(`docs/superpowers/plans/2026-10-07-raw-input-keys.md`).

Not done yet: ServerQuery browser (v1.1), tabs for several private chats (one is shown at a
time), permissions UI, channel create/edit, file browser and avatars, keeping the microphone
on one server while looking at another,
the old CELT codec (see below), pre-3.1 servers (`initivexpand`),
hotkeys other than talk, whisper and mute keys, game controller buttons, and an overlay inside
games that run in exclusive full screen (the speaking window is an ordinary window on top).

Not yet verified by anyone: a conversation or a whisper with the **official** TS3 client
(everything so far is PhishSpeak ↔ real server ↔ PhishSpeak), and voice on a real
internet server (signing in to one has worked). Do that first before trusting it for daily use.
Also unverified: sound by ear (which includes the event sounds, noise suppression and automatic
gain), a talk key held on a real keyboard or mouse (the checks pressed F13 to F24 by program),
keys while a game running as administrator has the focus, two different servers at once (the
multi-server tests used two connections to one server), the speaking window over a game, the
look with any renderer but the software one, a window dragged between two displays with
different scales (both displays of this PC have the same one, so that change was made from
reading the toolkit's code), and SRV and TSDNS lookups against a domain that publishes such
records (the tests feed made-up answers), a `ts3server://` link clicked in a browser, and
Speex made by a TeamSpeak client. Echo cancelling has been measured on simulated rooms
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
| `ps-client` | Connection actor thread: handshake, ack/resend, ping, command dispatch, channel/client/group book, voice and whispers in/out, events; `spacer` recognises spacer channels, `filetransfer` fetches icons over the server's file port, `resolve` finds a server through SRV, TSDNS or its plain name | done, 33 tests + live tests |
| `ps-oldcodecs` | Speex decoder (8, 16 and 32 kHz) in safe Rust, no dependencies | done, 27 tests + 3 run by hand |
| `ps-voice` | Opus codec, Speex playback at 48 kHz, resampler, jitter buffer + mixer (talkers keyed by connection and client, a volume per talker), VAD/PTT gate, lanes (which key is held decides where a frame goes), echo canceller (`echo.rs`), noise suppression (`denoise.rs`), automatic gain (`agc.rs`), event sounds (`cues.rs`), cpal device I/O (WASAPI) | done, 85 tests + live tests |
| `ps-app` | The windows. `session.rs` one connection (events, tree rows and folding, chat history, reconnecting), `app.rs` all sessions, the viewed one and where the microphone goes, `mic.rs` what each server is told about the microphone and when, `app/shortcuts.rs` choosing keys, the whisper key editor and the lane table, `hotkeys.rs` key combinations and what counts as held, `keywatch.rs` the thread that reads the keys, `whisper.rs` whisper keys and their file, `speakers.rs` who is listed in the speaking window, `scale.rs` keeping a window's size across displays, `links.rs` reading `ts3server://` links and who opens them, `instance.rs` handing a second start over to the first, `bookmarks.rs`, `settings.rs`, `platform.rs`, `ui/` theme, widgets, main, settings and speaking windows, `icons.rs` checks, shrinks and caches icons | done, 122 tests + live tests |
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

### One microphone, several servers

- `client_input_hardware` says whether the client has a microphone open for this connection. The
  TeamSpeak client lets one server tab own the capture device (its changelog: "Activate
  Microphone" per tab, "Activate microphone automatically" when switching tabs, "the server tab
  which previously owned the capture device"), and the other tabs carry 0.
- Measured on 3.13.8: while it is 0 the server passes on none of that client's voice (0 of 200
  packets in four seconds, the end-of-talk packet included) and everyone in view is told of a
  change at once (1 ms). The client gets its own change back as `notifyclientupdated`, so its own
  entry shows what the server holds.
- Flood protection, measured with the settings a server starts with
  (`virtualserver_antiflood_points_tick_reduce=5`, `..._points_needed_command_block=150`): a
  `clientupdate` costs 15 points and 5 drain each second. Of 11 sent at once the 11th was refused
  (`error id=524 msg=client is flooding extra_msg=retry in 5999ms`); at one every two seconds the
  29th was refused, after 56 s. A refused command is charged as well (one sent 1.6 s after a
  refusal was told 7394 ms). A command sent once the named time has passed was accepted, both
  times it was tried. The refusal carries the `return_code` of the command it refuses.

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

### Old codecs

- A voice packet names its codec: 0, 1 and 2 are Speex at 8, 16 and 32 kHz, 3 is CELT, 4 and 5
  are Opus. A channel has one codec, and a TeamSpeak client talks in its channel's codec.
- Where the old ones still exist (the server's own changelog): from server 3.7.0 a channel can
  no longer be set to Speex or CELT, and server 3.11.0 (15 January 2020) turns every channel
  into Opus. So Speex and CELT are only met on servers older than that. The free licence built
  into 3.6.1 and 3.10.2 has run out and they no longer start, so no server with such channels
  could be run here.
- The current server does not look at the codec number: packets marked 0, 1, 2 and 3 sent into
  an Opus channel reached a listener unchanged (100 of 100 each).
- Speex as TeamSpeak sends it is one frame every 20 ms, written as the Speex library writes a
  frame, at a fixed size per quality, the channel's quality number being Speex's own. This is
  not from a capture; it follows from the bandwidth figures the TeamSpeak client shows, which
  are 50 packets a second of the frame plus 45 bytes: 2.49 and 5.22 KiB/s (narrowband, quality
  0 and 10), 2.69 and 7.37 (wideband), 2.73 and 7.57 (ultra-wideband). The reference encoder
  makes frames of 6 and 62, 10 and 106, 11 and 110 bytes at those settings, which gives exactly
  those six figures.
- With a channel's "latency factor" above 1 a packet carries several frames. How TeamSpeak packs
  them is not known. The decoder takes both ways (one after another bit to bit, as the Speex
  library does it, or each padded to whole bytes) and only frames of the same kind as the first.
- CELT is not played, by decision (2026-10-07). TeamSpeak replaced its CELT on 10 May 2011
  (client 3.0.0-rc1: "Updated CELT codec. Due to codec bitstream incompatibility you can only
  communicate with new clients"), which points to CELT 0.11. Measured with the four 0.11
  releases built from source: 0.11.1 and 0.11.2 produce identical bytes; 0.11.3 is the same
  with one byte put in front of every frame (0x10 for a 10 ms frame, 0x18 for 20 ms); 0.11.0
  differs from the others (22 dB apart); and decoding with the wrong one gives noise thousands
  of times louder than full scale. Which release TeamSpeak ships, and whether it uses 10 or
  20 ms frames, is not published; the client's bandwidth figures (6.10 and 13.92 KiB/s) fit 50
  packets a second of 80 and 240 bytes either way.

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
- One microphone with several servers, live: one PhishSpeak with two connections to the test
  server at its default flood settings, a `channeltest` listener in the channel, and ServerQuery
  reading what the server holds. The connection not viewed was reported off about 2 s after it
  connected. On switching, the end-of-talk packet reached the channel, the new connection was
  reported on 1 ms later and its first voice packet came 6 ms after that; the one left behind
  was reported off 2.0 s later. Six switches 0.7 s apart cost one report during the switching,
  no packet was lost (50 a second throughout, counted per second), and the report that was then
  due came 15.0 s after the previous one. Muting showed both as muted on the server while the
  list kept marking where the microphone is. Leaving the server that had the microphone gave
  it to the other in the same tenth of a second. After the server was restarted both
  connections came back by themselves, the viewed one on and the other already off when it was
  first seen; the same with two bookmarks set to connect at start. With the server set to
  refuse a second command within two seconds, the "on" report was refused: PhishSpeak showed
  its own name as muted and "Microphone not on here yet", sent nothing more until the time the
  server named had passed, then sent it once and was heard 4.8 s after the switch (twice the
  same). Before that rule existed, a retry every 3 s was refused each time, because the server
  charges refused commands too, and the connection stayed silent until the limit was raised
  again 9 s later.
  Not done: two different servers, and what the TeamSpeak client shows for such a connection.
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
- 2026-10-07, live against the test server with clicks and keys sent by program: the person
  panel (a private message each way, a poke, volume and mute for one person, away); a server
  stopped and started again, after which the client came back by itself into the channel it had
  been in; a privilege key used from the menu; asking to talk in a moderated channel; the
  channel panel with topic and description on right-click; the host message at sign-in; the
  mute keys (F13 and F14 pressed by program); a GIF icon shown as a still picture.
- The release workflow: tag `v0.1.0` built on GitHub Actions and published an installer, a zip
  and checksums. The checksums matched after download; the installer installed, started and
  uninstalled on this PC.
- The speaking window, with a second client talking in the channel: a name appears lit while
  its owner talks, stays dimmed for the set time after they stop (the wait starts when they
  stop, however long they talked) and then goes; moving, sizing by the corner down to the
  smallest size and up again, locking and unlocking from the window, the menu and settings;
  locked, the window lets clicks through and is fully transparent while nobody is listed;
  "above other windows", the see-through slider and "list everyone" take effect at once; two
  connections give a heading each; a box too small for everyone shows who fits and "+N"; place
  and size survive a restart; closing the main window ends the program with the speaking
  window open; and the window did not become the active window when it was shown from the menu,
  clicked, dragged or locked. Moving and sizing were done with mouse messages sent by program.
- Links, with a stand-in scheme so that the PC's own `ts3server` entry was never touched
  (`PHISHSPEAK_LINK_SCHEME=ts3server-test`): the switch wrote the per-user entry and Windows
  named PhishSpeak as the program it would run for such a link; a second start with a link
  handed it over and left, and the first one showed the dialog with the address, the nickname
  and the line about what the link carried; after Connect the client was in the channel the
  link named, the server had put it in the group of the privilege key the link carried, and
  the bookmark the link asked for was saved with that channel and without any password; a
  plain second start left again with one program still running; switching off removed the
  entry, and so did `--forget-links`, which also set the switch off in the settings file.
  Not done: clicking a real link in a browser, and the real `ts3server` entry.
- The published 0.3.0 installer, in a scratch folder: it installed, and uninstalling it removed
  a stand-in links entry that pointed at the installed program and set the switch off in the
  settings file, so the uninstaller does give links back.
- The published 0.3.1 program, taken from the zip and run against the test server: both
  published files match their checksums; the connection not viewed was reported off 2.0 s after
  it connected, and on switching the other was reported on in the same millisecond as the
  end-of-talk packet and the one left behind off 2.0 s later.
- Speex. The decoder's output is the same, sample for sample, as that of the reference library
  (libspeex 1.2.1 built without SSE) on 60 streams, 6.7 million samples: every quality from 0 to
  10 in all three kinds, changing bit rate, silence, several frames in a packet, and lost
  packets filled in. 1.2 million broken packets (random bytes, real packets with bits flipped,
  cut short, glued together or with bytes pushed in) caused no fault and no sample outside the
  range. Decoding costs 0.04, 0.07 and 0.10 % of one processor for the three kinds. Live: real
  Speex streams sent from one PhishSpeak through the TeamSpeak 3.13.8 server to another arrived
  complete and in order and decoded to the reference sound (narrowband, wideband,
  ultra-wideband, and three frames to a packet), and came out of the mixer at 48 kHz at the
  same level; in the window the talker lit up and no "cannot play" notice appeared, while a
  packet marked CELT brought up the notice that names CELT.
  Not done: Speex made by a TeamSpeak client, and whether TeamSpeak clients in a Speex channel
  play the Opus that PhishSpeak sends there.

Dev tools (examples): `cargo run -p ps-client --example probe -- <host> [--identity file] [--say TEXT]
[--join CID] [--loss 0.2] [--auto-level] [--log]`, `cargo run -p ps-voice --example voicetest --
<host> [--music] [--listen] [--burst 70000] [--loss 0.2]`, `cargo run -p ps-voice --example
devicetest -- [--input NAME] [--tone]`, `cargo run -p ps-voice --example channeltest -- <host>
[--nick NAME] [--join CID] [--seconds N] [--talk SECONDS] [--whisper client:ID|channel:ID|commanders|everyone]
[--commander] [--codec N] [--frames FILE] [--frame-ms N] [--save DIR] [--speex]
[--speex-reference FILE] [--mic-off] [--mic SECONDS:on|off]` (sits in one channel and reports
every voice, whisper and end packet it hears, their sizes and spacing, the sound formats, and
whose microphone is reported off or muted; with `--talk` it also sends a tone,
as a whisper with `--whisper`; `--codec` writes another codec number on what it sends and
`--frames` sends ready-made packets from a file; `--save` keeps every packet heard, which is
the way to study a voice format nobody has described; `--speex` decodes the Speex it heard
and `--speex-reference` compares that with a file of samples; `--mic-off` signs in with the
microphone reported off and `--mic` reports it on or off later, neither of which stops
`--talk`, which is how to see what a server does with such a client),
`cargo run -p ps-voice --example echotest -- [--output NAME] [--input NAME | --loopback]
[--seconds N] [--level DB]` (plays a speech-like test sound and reports how loudly the input
hears it with echo cancelling off and on, when the sound came back and the clock difference;
it is audible unless the output is a virtual device),
`cargo run -p ps-client --example whispertest -- <host> --booth CID --drift CID` (re-runs the
who-hears-what table and fails if a server behaves differently).
`PHISHSPEAK_TRACE=1` makes the GUI show every command in the chat drawer.
`PHISHSPEAK_LINK_SCHEME=<name>` makes the links switch and the link reader use another scheme
than `ts3server`, so that links can be tried without touching the PC's real entry.
`probe` also takes `--icon ID` (repeatable), `--all-icons`, `--save DIR`, `--ft-port N`,
`--voice` (how each talker's stream ends: packet sizes and timing, no sound), `--token KEY`
(use a privilege key), `--send COMMAND` (repeatable), `--nick NAME` and `--seconds N`.
`cargo run -p ps-client --example resolve -- <address>` shows where an address leads and by
which of the three lookups.

Scripts in `tools/`: `hold_keys.py F13..F24[+F13..F24] <seconds>` presses keys no keyboard has,
for checking talk and whisper keys; `seed_whisper_tree.py --password <query password>` (run
where the test server's query port is reachable) makes the Booth and Drift channels and lets
guests be channel commanders; `seed_test_icons.py --password <query password>` puts good and
deliberately bad icons on the test server (`--count N`, `--upload FILE`, `--assign WHAT=ID`,
`--clear`); `speex_vectors.py build|signals|streams|pack|full --work DIR` makes the Speex
reference streams from libspeex 1.2.1 (`build` and `streams` need gcc and make, `signals` and
`pack` need numpy; `full` puts the whole set where the by-hand test looks for it);
`package_release.py [--tag vX.Y.Z] [--skip-installer]` builds the
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
- The servers without the microphone are told so (`mic.rs`, one `Report` per connection, stepped
  on every change of view and every 33 ms). "On" is sent at once and before the microphone is
  handed to the connection, because the server drops voice until it has it. "Off" is sent when
  the microphone has been away for 2 s and at most every 15 s per server: a short look at
  another server costs nothing, and switching back and forth, however fast or regular, never
  holds more than 30 of the 150 points a default server allows (a test runs six rhythms for
  ten minutes against the measured point counts). What was sent is believed for 3 s; after
  that the server's own record counts and a difference is sent again, at twice the wait each
  time up to 30 s. When the server answers that we are flooding, that connection's report
  waits the time the server names plus 250 ms, because trying earlier is refused and charged.
  A refused "off" still waits out its 15 s: it changes only what others see, and the points
  are better left for an "on". From a flooding answer until the server's record agrees, the
  server's record is what is shown: your own name carries the muted mark, you are not shown as
  talking, the status line says "Microphone not on here yet" and the list does not mark that
  server as having the microphone. A server that never repeats our own state back is not taken
  for a refusal. A connection signs in with the microphone on if it is about to be viewed and
  off if it reconnects in the background or is one of several bookmarks connecting at start
  that will not be the one shown, so none of them needs a report afterwards.
- While the server you are looking at is still connecting, no server has the microphone: the
  one you came from is told off after the 2 s and on again when you return or the attempt fails.
- The mark in the list of connected servers says where the microphone is, muted or not; mute
  has its own button. With one server connected there are no marks.
- Not built: keeping the microphone on one server while looking at another (the TeamSpeak
  client's "Activate microphone automatically" switched off). Known gap: a mute or unmute that
  the server refuses for flooding is not sent again; the tree then shows the server's state.
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
  rewritten as `talk_key=<key codes joined with +>`. A second way of reading keys, in which
  Windows reports each key as it moves (Raw Input, as Mumble does), is planned as a switch and
  was tried in a throwaway copy: `docs/superpowers/plans/2026-10-07-raw-input-keys.md`.
- Mute keys (`mute_mic_key`, `mute_sound_key`) are combinations like talk keys and act once per
  press. A key still held from choosing it does not fire.
- The speaking window (`ui/speakers.slint`, `speakers.rs`) is an ordinary top-level window
  without a frame, not something drawn inside a game: it shows over programs that run in a
  window or a borderless window, not over exclusive full screen. It lists the people talking in
  your channel on every connected server, people whispering to you from elsewhere, and yourself
  while you send; with more than one server each gets a heading. A name stays, dimmed, for
  `speakers_linger` seconds after its owner stops (10 by default, 0 to 60), counted from the
  moment they stop. Unlocked it has an outline, a corner to size it and buttons to lock and
  hide it; locked it passes clicks through and is fully transparent while it lists nobody.
  Transparency is for the whole window (20 to 100 %). The window is marked so that clicking it
  never makes it the active window, and showing it hands the focus back to the window that had
  it. Place and size are kept in `settings.ini`. It has a taskbar button of its own.
- Per-person volume and mute are kept by the person's UID (`voice.<uid>=<percent>[,muted]`, 256
  people at most) and applied whenever that person is seen. The slider is squared before use,
  so half way is a quarter of the power.
- Reconnecting: only a connection that had been up is retried, after 2, 4, 8, 15 and then every
  30 seconds. The password in use and the channel you were in are kept for the retry; a refusal
  that trying again cannot cure (a ban, a wrong password) stops it.
- A server address without a port is tried as an SRV record `_ts3._udp.<name>`, then through
  TSDNS (an SRV record `_tsdns._tcp` or port 41144 on the domain), then as a plain name; with a
  port typed, the SRV step is skipped. The lookups share a 4 second limit.
- Event sounds are short tones made in code, no sound files, mixed into the output after the
  voices with their own volume (`cue_volume`).
- Noise suppression lowers each frequency band by how much steady noise it holds and delays
  your voice by 4 ms. Automatic gain steers speech towards -20 dB, between -6 and +24 dB, and
  does not turn up while you are silent. The order is echo cancelling, noise suppression,
  automatic gain, then the microphone boost.
- When a window lands on a display with another scale, the toolkit keeps the smallest allowed
  size in the old display's pixels. PhishSpeak has the limits worked out again and then puts
  the window back to the size it had, measured in the new scale (`scale.rs`).
- Links (`links.rs`, `instance.rs`). A link is `ts3server://host[:port]` with the optional parts
  TeamSpeak documents: `port`, `nickname`, `password`, `channel`, `cid`, `channelpassword`,
  `token`, `addbookmark` (a `cid` wins over a `channel`; a `+` stays a plus sign). A link is
  refused when it names no server, hides the server behind an `@` or an escaped character,
  contains a line break or other control character, or is longer than 2048 characters.
  A link never connects by itself. It opens the connect dialog with the address, the nickname
  and a server password filled in, and a line that says what else it carries (a channel, a
  channel password, a privilege key, a bookmark name). Those extras are used only if the address
  is still the link's when Connect is pressed. A link to a server that is already open only
  shows it; a link to a server that has a bookmark uses that bookmark's identity and does not
  change the bookmark. Passwords from a link are used once and never saved.
- Who opens links is a switch (Settings, Bookmarks), off by default. On writes
  `HKCU\Software\Classes\ts3server` for the signed-in user only, no administrator rights, and
  remembers the command that was there (`links_previous` in `settings.ini`); off puts that
  command back, or removes the entry when there was none, so a machine-wide entry of another
  program counts again. If another program has taken the links in the meantime, PhishSpeak
  leaves them alone and the switch goes off. If PhishSpeak's own file has moved, the entry is
  pointed at the new place at the next start. `PhishSpeak.exe --forget-links` does the same as
  switching off and is what the uninstaller runs.
- One PhishSpeak per profile. A second start hands its link or `--connect` to the first and
  leaves; with nothing to hand over it brings the first one's window to the front. The first one
  listens on a loopback port and writes the port and a random word to
  `%APPDATA%\PhishSpeak\instance`; only a program that can read that file is listened to, so a
  web page cannot talk to the port. If the hand-over is not answered within two seconds the
  second start carries on as a program of its own.
- A whisper key opens the microphone by itself whatever "Send my voice" says, acts on the server
  being viewed, and never falls back to the channel: if its targets are gone, offline or on
  another server, the frames are dropped and the dock says why. Whispers are always Opus Voice,
  and a frame is encoded into the room the target list leaves in the packet (30 channels and 60
  people at most, 122 bytes left). Every audience gets its end marker when the stream to it
  stops: key released, another key pressed, mute, or a change of viewed server.
- Speex is decoded by PhishSpeak's own code in `ps-oldcodecs`, a rewrite in Rust of the decoder
  of libspeex 1.2.1 (floating point). The crate forbids `unsafe`, has no dependencies, and
  never trusts a packet: a frame that names something that does not exist, or reads past the
  end, is refused and leaves the decoder as it was; requests and user data inside the stream
  (which TeamSpeak never sends) are refused too; if the decoder's memory ever holds a value
  that is not a number it starts over. The enhancer is on, as in the reference. Each talker's
  Speex is brought to 48 kHz by the resampler the devices already use (which holds back 15
  samples, 2 ms at 8 kHz), and the first lost packet is filled in by Speex itself. A packet
  that decodes louder than any voice (RMS above 0.7) is treated as lost.
- PhishSpeak always talks in Opus, also in a channel set to Speex or CELT: the packet says what
  it carries, the server passes it on, and every TeamSpeak client since 3.0.10 (2013) has Opus.
  That those clients play it in such a channel is an assumption.
- The Speex test streams in `ps-oldcodecs/tests/data` come from the reference library through
  `tools/speex_vectors.py` and `tools/speexref.c`; the tool reproduced them byte for byte.
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
14. ✅ SRV and TSDNS lookup, GIF icons, the person panel with private messages, poke and
    per-person volume, reconnecting, privilege keys, asking to talk, the channel panel, event
    sounds, noise suppression, automatic gain, mute keys; the first release built by the
    workflow (0.1.0).
15. ✅ The speaking window; windows keep their size across displays with different scales.
16. ✅ `ts3server://` links as a setting, one PhishSpeak per profile; Speex from old channels.
17. ✅ With several servers, the ones without the microphone are told it is off.
18. Next: test against the official client and a public server; try echo cancelling, noise
    suppression and the event sounds by ear; reading keys through Raw Input (planned);
    avatars.

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
