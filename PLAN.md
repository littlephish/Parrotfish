# PhishSpeak — Build Plan

A TeamSpeak 3 client in Rust with a Slint GUI. It logs in with real TS3 identities (P-256),
joins a server, shows the channel tree, chats, and does voice (Opus) both ways.

## Status (2026-10-06)

Working end to end against a real TeamSpeak 3.13.8 server: login, channel tree, channel
switching (incl. password channels), text chat, microphone capture → Opus → server, and
server → Opus → speakers, with and without voice encryption. 105 unit tests green.

Run it:

```
cargo run --release -p ps-app                      # GUI
cargo run --release -p ps-app -- --connect host[:port] [--nickname NAME]
```

Not done yet: ServerQuery browser (v1.1), whispers (send), private-chat tabs, permissions UI,
channel create/edit, file transfer/avatars, SRV/TSDNS lookup (use `host:port`), legacy
Speex/CELT codecs (reported in the log, not decoded), pre-3.1 servers (`initivexpand`).

Not yet verified by anyone: a conversation with the **official** TS3 client in the same
channel (everything so far is PhishSpeak ↔ real server ↔ PhishSpeak), and a real internet
server. Do that first before trusting it for daily use.

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
| `ps-identity` | INI parse, identity (de)obfuscation, DER, P-256, UID, hashcash level, sign/verify, generate/save | done, 18 tests |
| `ps-crypto` | EAX-AES128 (8-byte MAC), dummy key, per-packet key/nonce, license chain, Ed25519 shared secret, RSA puzzle | done, 16 tests |
| `ps-protocol` | Packet headers, command escape/parse/build, QuickLZ + fragmentation, receive windows/generations, Init1 payloads, voice payloads | done, 32 tests |
| `ps-client` | Connection actor thread: handshake, ack/resend, ping, command dispatch, channel/client book, voice in/out, events | done, 7 tests + live tests |
| `ps-voice` | Opus codec, resampler, jitter buffer + mixer, VAD/PTT gate, cpal device I/O (WASAPI) | done, 28 tests + live tests |
| `ps-app` | Slint GUI wiring everything together, settings, PTT hotkey | done, 4 tests |
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
- Audio devices on this PC: capture runs at 48 kHz from the Arctis and webcam mics; a −48 dBFS
  test tone pushed through the playback path was read back from the headphone endpoint via
  WASAPI loopback at −48.0 dBFS.

Dev tools (examples): `cargo run -p ps-client --example probe -- <host> [--identity file] [--say TEXT]
[--join CID] [--loss 0.2] [--auto-level] [--log]`, `cargo run -p ps-voice --example voicetest --
<host> [--music] [--listen] [--burst 70000] [--loss 0.2]`, `cargo run -p ps-voice --example
devicetest -- [--input NAME] [--tone]`. `PHISHSPEAK_TRACE=1` makes the GUI log every command.

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
- Settings live in `%APPDATA%\PhishSpeak\settings.ini`; identities created in the app in
  `%APPDATA%\PhishSpeak\identities`. An improved key offset for an imported identity is cached
  in settings, the imported `.ini` is never modified. Server passwords are not stored.

## Milestones

1. ✅ `ps-identity`: decode, verify, UID.
2. ✅ `ps-app` Slint shell.
3. ✅ UDP framing + Init1 against a local TS3 server.
4. ✅ Crypto handshake + `clientinit` → `initserver`; client appears in a channel.
5. ServerQuery browser (TCP 10011) feeding the server field.
6. ✅ Voice: Opus encode/decode + device I/O, send/receive `Voice`.
7. ✅ Text chat + channel tree (basic). Private chats, pokes UI, permissions: open.
8. 🚧 Identity management (create ✅ / import ✅), settings ✅, polish.
9. Next: test against the official client and a public server; SRV/TSDNS resolution; whispers;
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

## Conventions

- No comments unless asked; follow existing code style per crate.
- Test/verification scripts in Python (not PowerShell).
- Each protocol struct gets a round-trip test with captured or TSLib-derived vectors.
