# PhishSpeak — Build Plan

A TeamSpeak 3 client in Rust with a Slint GUI. It logs in with real TS3 identities (P-256),
joins a server, shows the channel tree, chats, and does voice (Opus) both ways.

## Status (2026-10-06)

Working end to end against a real TeamSpeak 3.13.8 server: login, channel tree, channel
switching (incl. password channels), text chat, microphone capture → Opus → server, and
server → Opus → speakers, with and without voice encryption. 120 unit tests green.

The window is the compact tree layout in the Twilight reef palette (design:
`docs/superpowers/specs/2026-10-06-compact-window-design.md`): spacer channels are drawn as
dividers, servers can be bookmarked, several servers can be connected at once (you hear all
of them, the microphone goes to the one you are viewing), and every setting lives in a
separate tabbed settings window.

Run it:

```
cargo run --release -p ps-app                      # GUI
cargo run --release -p ps-app -- --connect <bookmark name or host[:port]> [--nickname NAME] [--channel NAME]
```

`--connect` can be given more than once; `--nickname` and `--channel` apply to the one before them.

Not done yet: ServerQuery browser (v1.1), whispers (send), private-chat tabs, permissions UI,
channel create/edit, file transfer/avatars, SRV/TSDNS lookup (use `host:port`), legacy
Speex/CELT codecs (reported in the log, not decoded), pre-3.1 servers (`initivexpand`).

Not yet verified by anyone: a conversation with the **official** TS3 client in the same
channel (everything so far is PhishSpeak ↔ real server ↔ PhishSpeak), and voice on a real
internet server (signing in to one has worked). Do that first before trusting it for daily use.
Also unverified: sound by ear, the hold-to-talk key, and two different servers at once (the
multi-server test used two connections to one server).

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
| `ps-protocol` | Packet headers, command escape/parse/build, QuickLZ + fragmentation, receive windows/generations, Init1 payloads, voice payloads | done, 32 tests |
| `ps-client` | Connection actor thread: handshake, ack/resend, ping, command dispatch, channel/client book, voice in/out, events; `spacer` recognises spacer channels | done, 10 tests + live tests |
| `ps-voice` | Opus codec, resampler, jitter buffer + mixer (talkers keyed by connection and client), VAD/PTT gate, cpal device I/O (WASAPI) | done, 29 tests + live tests |
| `ps-app` | The windows. `session.rs` one connection (events, tree rows, chat history), `app.rs` all sessions, the viewed one and where the microphone goes, `bookmarks.rs`, `settings.rs`, `platform.rs` (talk key), `ui/` theme, widgets, main and settings windows | done, 19 tests + live tests |
| `ps-serverquery` | Text protocol over TCP 10011 | not started |

Threads: UI thread (Slint, 33 ms timer drains client events) · `ps-client` actor + UDP reader ·
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
  talk spurt. Voice id = the Voice packet id.
- Encrypt voice when `virtualserver_codec_encryption_mode` is 2, or 0 and the channel has
  `channel_codec_is_unencrypted=0`; otherwise send with the Unencrypted flag + SharedMac.
- Multi-item notifications (`a=1 b=2|b=3`) inherit missing keys from the first item.

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

Dev tools (examples): `cargo run -p ps-client --example probe -- <host> [--identity file] [--say TEXT]
[--join CID] [--loss 0.2] [--auto-level] [--log]`, `cargo run -p ps-voice --example voicetest --
<host> [--music] [--listen] [--burst 70000] [--loss 0.2]`, `cargo run -p ps-voice --example
devicetest -- [--input NAME] [--tone]`, `cargo run -p ps-voice --example channeltest -- <host>
[--nick NAME] [--join CID] [--seconds N] [--talk SECONDS]` (sits in one channel and reports
every voice and end-of-talk packet it hears; with `--talk` it also sends a tone).
`PHISHSPEAK_TRACE=1` makes the GUI show every command in the chat drawer.

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
  is cached in settings. Passwords are used for one attempt and never written to disk.
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
9. Planned, not started: the icons a server defines for channels, people, groups and itself
   (`docs/superpowers/plans/2026-10-06-custom-icons.md`, waiting for review).
10. Planned, not started: talk keys of your choice and TeamSpeak-style whisper keys
    (`docs/superpowers/plans/2026-10-06-talk-and-whisper-keys.md`, waiting for review).
11. Next: test against the official client and a public server; SRV/TSDNS resolution;
   noise suppression / AGC; per-user volume in the UI (the mixer already supports it).

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

## Conventions

- No comments unless asked; follow existing code style per crate.
- No real identities, UIDs or machine paths in tests, documents or scripts. Use the keys
  tsclientlib publishes, or generate one in the test.
- Test/verification scripts in Python (not PowerShell).
- Each protocol struct gets a round-trip test with captured or TSLib-derived vectors.
