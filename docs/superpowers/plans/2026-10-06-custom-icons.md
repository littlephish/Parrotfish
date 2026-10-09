# Custom Icons Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show the icons a TeamSpeak server defines: on channels, on people (their own icon and the icons of their groups) and for the server itself, downloaded from the server and cached.

**Architecture:** `ps-client` reads icon ids and group lists out of messages it already receives, and gains a small download path: the request goes over the voice connection, the bytes come over TCP. `ps-app` gets an icon store that checks, caches (per server) and hands images to the tree rows. Icons are requested only when a row needs them, one at a time, slowly enough that a server's flood protection never trips.

**Tech Stack:** Rust stable, Slint 1.18, existing workspace crates. No new dependencies for PNG and JPEG icons, which is what this build of Slint can decode (`image` with only `png` and `jpeg`; checked with `cargo tree`). GIF needs a decision, see below.

**Spec:** none yet. Nothing here is built. The next two sections are the scope and the decisions I made for you; change them before anyone starts.

**Status:** Tasks 1 to 5 are built and checked (2026-10-06); what changed on the way is in `2026-10-06-custom-icons.ledger.md`. Task 6 (GIF) was built on 2026-10-07.

**How to read the tasks:** tests and the functions that guard against bad input are given in full. Steps that connect existing code are described by what they must do and are held to the tests and live checks listed with them.

## What this builds

```
+------------------------------------------+
| [RR] Reef Runners (s)            [NS]  v |  (s) the server's icon, after its name
|      12 people online                    |
+------------------------------------------+
|  ))  Lobby                      (c)   3  |  (c) the channel's icon, before the count
|        o Minnow              (g)(g)(p)   |  (g) group icons, (p) the person's own icon
|        * Marlin          (g)  (mic-off)  |  icons sit left of the mute marks
|  [lock] Squad Alpha             (c)      |
+------------------------------------------+
```

- A channel with an icon shows it at the right of its row.
- A person shows up to four icons at the right of their row: channel group, then server groups, then their own.
- The server's icon appears after its name in the header.
- TeamSpeak's five standard group icons (channel admin, operator, server admin, query admin, voice) are drawn for Parrotfish, because servers refer to them by number and never send them.
- Every other icon is downloaded once, kept in `%APPDATA%\Parrotfish\cache\icons`, and loaded from there on later runs.
- A row never waits for an icon. If an icon cannot be fetched the row simply has none.

## Decisions I made that you have not confirmed

1. **Order on a person's row:** channel group, server groups (in the server's sort order), own icon. At most four; extra ones are not shown.
2. **Where the server icon goes:** after the server name in the header. The tiles keep their initials, because a 16 px icon stretched over a 34 px tile looks blurred.
3. **PNG and JPEG only in Tasks 1 to 5.** That is what the app can decode today. Some servers use GIF icons, so Task 6 adds them (first frame only) by using the `gif` crate directly. That crate is already compiled into the app for SVG rendering, so the build does not grow, but it becomes a dependency Parrotfish names itself. Drop Task 6 if you do not want that. BMP and SVG icons from servers are not shown.
4. **Not in this plan:** country flags, myTeamSpeak badges, avatars, group names shown beside nicknames, a tooltip naming the group, and uploading or managing icons. Avatars would reuse the download path built here.
5. **No on/off switch.** Icons are always shown. Say so if you want a setting.
6. **Limits:** an icon file may be at most 512 KiB and 256 x 256 pixels; anything else is ignored. At most 500 icons are kept per server.
7. **Stale icons:** a cached icon is fetched again after 7 days, because a server can replace the picture behind a number.

## Facts this plan rests on

Checked on 2026-10-06 against the local TeamSpeak 3.13.8 test server and the reference projects (tsdeclarations `Messages.toml`, tsclientlib, TSLib).

- Ids arrive in fields Parrotfish already receives and ignores: `channel_icon_id` (`channellist`, `notifychannelcreated`, `notifychanneledited`), `client_icon_id`, `client_servergroups` (comma-separated) and `client_channel_group_id` (`notifycliententerview`, `notifyclientupdated`), `virtualserver_icon_id` (`initserver`, `notifyserveredited`).
- The server sends `notifyservergrouplist` and `notifychannelgrouplist` at sign-in without being asked. Each item has `sgid` or `cgid`, `name`, `iconid`, `sortid`.
- Group changes while connected: `notifyservergroupclientadded` / `notifyservergroupclientdeleted` (`sgid`, `clid`), `notifyclientchannelgroupchanged` (`cgid`, `cid`, `clid`).
- **The same icon is written two ways.** With id 2154984321 on a channel, `channellist` reported `2154984321` and the permission list reported `-2139982975`. tsdeclarations adds that some servers send it as a 64-bit number. All three must become the same 32-bit id.
- Ids 100, 200, 300, 500 and 600 belong to the standard groups and have no file on the server.
- **An id is only a file name.** The server accepted and listed `icon_12345` whose content has a different CRC32, so the client must not require the id to match the content.
- Download: `ftinitdownload clientftfid=<n> name=/icon_<id> cid=0 cpw seekpos=0 proto=1`. The answer is `notifystartdownload clientftfid serverftfid ftkey port size proto` (this server sent no `ip`). Then: open TCP to the server on `port` (30033 here), send the 32 characters of `ftkey`, read exactly `size` bytes. A 92-byte icon came back identical.
- A missing icon is answered with `notifystatusfiletransfer clientftfid status=2054 msg=invalid\sfile\spath size=0`.
- Icons are set through the permission `i_icon_id` (`channeladdperm`, `servergroupaddperm`, `clientaddperm`); `channeledit channel_icon_id=` is refused. The server icon is `serveredit virtualserver_icon_id=`.
- `Command::build` writes an empty value as a bare key and escapes `/`, so the request above is what the existing builder produces.
- The test server now holds two icons from this check: `2154984321` (on Deep Rock, Squad Alpha and as the server icon) and `452340182` (on Tide Pool and on the Guest server group), plus the stray `icon_12345`. Its flood protection is still relaxed from earlier tests (`tick_reduce=10000`, both thresholds `1000000`); the defaults are 5, 150 and 250.

Not checked yet, and settled in Task 2: how many flood points one `ftinitdownload` costs a normal client.

## Global Constraints

- No new crates in Tasks 1 to 5. Task 6 adds `gif` as a direct dependency of `ps-app` and nothing else.
- No TeamSpeak artwork is copied. The five standard group icons are drawn for this project as 24 x 24 stroke-only SVG files like the existing ones.
- Icon bytes are untrusted. Size and pixel dimensions are checked from the file header before anything is decoded. Limits: 512 KiB, 256 x 256. SVG from a server is never rendered.
- The download connects only to the address of the voice connection, on the port the server names. An address named in a reply is ignored.
- One download at a time per connection, at least 500 ms between requests, 5 s to connect, 10 s in total. A flood warning from the server pauses icon requests for 15 s.
- Downloads never run on the connection thread or the UI thread. Voice must be unaffected.
- Icons are drawn 16 x 16 in rows that stay 26 px high. The space for an icon is reserved while it is on its way, so a row does not shift when the picture arrives.
- Cache files live under `%APPDATA%\Parrotfish\cache\icons\<server>\`, named only from digits and hex. Nothing from the server is used as a path.
- Amber keeps its three meanings; downloaded icons are never tinted.
- No comments in code. No real identities, UIDs or machine paths in tests, documents or scripts.
- Existing tests keep passing (120 today).

## Review Focus

Conditions the scope implies that are most likely to bite, each pinned to a task:

1. The same icon id arriving as `2154984321`, `-2139982975` and `18446744071569568641`: one icon, one file, one request. (Task 1 unit test.)
2. A server with dozens of icons and default flood protection: signing in must not get the client blocked or banned, and the file port being closed or filtered (common on hosted servers) must not slow the tree, the chat or voice. (Task 2: timeout test, live flood check, live closed-port check.)
3. A hostile or broken file: 600 KiB of data, a 33-byte PNG header claiming 20000 x 20000 pixels, a truncated file, HTML instead of an image. None may be decoded, crash the app or allocate more than the file. (Task 3 unit tests.)
4. Icons changing while connected: a channel gets an icon, a person joins a group, an icon is removed. The row follows without reconnecting. (Task 1 unit test, Task 5 live check.)
5. Two servers using the same id for different pictures: each shows its own. (Task 3 unit test on cache locations.)

## File Structure

| File | Responsibility |
|---|---|
| `ps-client/src/book.rs` (modify) | Icon ids on channels, people and the server; group lists; the icon list of each person |
| `ps-client/src/filetransfer.rs` (new) | Download command, reply parsing, the TCP download |
| `ps-client/src/conn.rs` (modify) | Dispatch the new messages; queue, pace and run icon downloads |
| `ps-client/src/lib.rs` (modify) | `ClientHandle::request_icon`, `Event::Icon`, exports |
| `ps-client/examples/probe.rs` (modify) | `--icon ID` (repeatable) and `--ft-port N` for live checks |
| `tools/seed_test_icons.py` (new) | Puts known icons on the local test server |
| `ps-app/src/icons.rs` (new) | Header check, limits, cache folder, per-icon state |
| `ps-app/src/session.rs` (modify) | Rows carry icon ids |
| `ps-app/src/app.rs` (modify) | Ask for missing icons, turn ids into images when publishing rows |
| `ps-app/ui/widgets.slint`, `main.slint`, `theme.slint` (modify) | Icon slots in rows, server icon in the header, the five standard icons |
| `ps-app/ui/icons/group-*.svg` (new) | The five standard group icons |

---

### Task 1: Icon ids and groups in `ps-client`

**Files:** Modify `ps-client/src/book.rs`, `ps-client/src/conn.rs`, `ps-client/src/lib.rs`.

**Interfaces:** Produces
`pub fn icon_id(raw: &str) -> u32`,
`pub struct Group { pub id: u64, pub name: String, pub icon: u32, pub sort: u32 }`,
`Channel::icon: u32`, `ServerInfo::icon: u32`,
`ClientInfo::icon: u32`, `ClientInfo::server_groups: Vec<u64>`, `ClientInfo::channel_group: u64`, `ClientInfo::icons: Vec<u32>` (filled by `Book::view`, in display order, no zeros, no repeats),
`Book::server_groups: HashMap<u64, Group>`, `Book::channel_groups: HashMap<u64, Group>`,
`Book::set_groups(&mut self, cmd: &Command, server: bool)`, `Book::group_member(&mut self, cmd: &Command, added: bool)`, `Book::channel_group_changed(&mut self, cmd: &Command)`.
`Group` and `icon_id` are exported from `ps-client/src/lib.rs` next to `Channel`.

- [x] **Step 1: Write the failing tests** in the `tests` module of `book.rs`:

```rust
#[test]
fn icon_ids_fold_to_32_bits() {
    assert_eq!(icon_id("0"), 0);
    assert_eq!(icon_id("100"), 100);
    assert_eq!(icon_id("2154984321"), 2154984321);
    assert_eq!(icon_id("-2139982975"), 2154984321);
    assert_eq!(icon_id("18446744071569568641"), 2154984321);
    assert_eq!(icon_id(" 452340182 "), 452340182);
    assert_eq!(icon_id(""), 0);
    assert_eq!(icon_id("icon"), 0);
}

#[test]
fn people_carry_group_and_own_icons() {
    let mut book = book_from(
        "channellist cid=1 cpid=0 channel_order=0 channel_name=Lobby channel_icon_id=0|cid=2 cpid=0 channel_order=1 channel_name=Deep\\sRock channel_icon_id=2154984321",
    );
    book.server.apply(&Command::parse("initserver virtualserver_name=Reef virtualserver_icon_id=-2139982975 aclid=3"));
    assert_eq!(book.server.icon, 2154984321);
    assert_eq!(book.channels[&2].icon, 2154984321);
    book.set_groups(
        &Command::parse("notifyservergrouplist sgid=6 name=Server\\sAdmin type=1 iconid=300 sortid=10|sgid=8 name=Guest type=1 iconid=452340182 sortid=20|sgid=9 name=Plain type=1 iconid=0 sortid=5"),
        true,
    );
    book.set_groups(
        &Command::parse("notifychannelgrouplist cgid=5 name=Channel\\sAdmin type=1 iconid=100 sortid=0|cgid=8 name=Guest type=1 iconid=0 sortid=0"),
        false,
    );
    book.own_id = 3;
    book.clients_entered(&Command::parse(
        "notifycliententerview cfid=0 ctid=1 reasonid=0 clid=3 client_nickname=Minnow client_type=0 client_servergroups=8,6,9 client_channel_group_id=5 client_icon_id=-2139982975|clid=4 client_nickname=Pike client_type=0 client_servergroups=8 client_channel_group_id=8 client_icon_id=0",
    ));
    let view = book.view();
    assert_eq!(view.channels[1].channel.icon, 2154984321);
    assert_eq!(view.client(3).unwrap().icons, vec![100, 300, 452340182, 2154984321]);
    assert_eq!(view.client(4).unwrap().icons, vec![452340182]);

    book.group_member(&Command::parse("notifyservergroupclientdeleted name=Server\\sAdmin sgid=6 invokerid=1 invokername=x clid=3 cluid=u"), false);
    book.channel_group_changed(&Command::parse("notifyclientchannelgroupchanged invokerid=1 invokername=x cgid=8 cgi=1 cid=1 clid=3"));
    assert_eq!(book.view().client(3).unwrap().icons, vec![452340182, 2154984321]);

    book.group_member(&Command::parse("notifyservergroupclientadded name=Server\\sAdmin sgid=6 invokerid=1 invokername=x clid=4 cluid=u"), true);
    book.clients_updated(&Command::parse("notifyclientupdated clid=3 client_icon_id=0"));
    book.upsert_channels(&Command::parse("notifychanneledited cid=1 reasonid=10 invokerid=1 channel_icon_id=452340182"));
    let view = book.view();
    assert_eq!(view.client(3).unwrap().icons, vec![452340182]);
    assert_eq!(view.client(4).unwrap().icons, vec![300, 452340182]);
    assert_eq!(view.channels[0].channel.icon, 452340182);

    book.set_groups(&Command::parse("notifyservergrouplist sgid=8 name=Guest type=1 iconid=0 sortid=20"), true);
    assert!(book.view().client(4).unwrap().icons.is_empty());
}
```

The second person in the sign-in message spells out all three icon fields on purpose: a multi-item message inherits missing keys from its first item.

- [x] **Step 2:** Run `cargo test -p ps-client icon`; expect a compile failure (`icon_id` not found).
- [x] **Step 3: Implement.**

```rust
pub fn icon_id(raw: &str) -> u32 {
    let text = raw.trim();
    if let Ok(value) = text.parse::<u64>() {
        return value as u32;
    }
    text.parse::<i64>().map(|value| value as u32).unwrap_or(0)
}
```

In `Channel::apply` read `channel_icon_id`; in `ServerInfo::apply` read `virtualserver_icon_id`; in `ClientInfo::apply` read `client_icon_id`, `client_channel_group_id`, and `client_servergroups` split on `,` into numbers (unparsable parts dropped). All ids go through `icon_id`.
`set_groups` replaces the whole map from the items of the message (`sgid` or `cgid`, `name`, `iconid`, `sortid`). `group_member` adds or removes `sgid` in the `server_groups` of client `clid`, without duplicates. `channel_group_changed` sets `channel_group` of client `clid` to `cgid`.
`Book::view` fills `icons` for every client:

```rust
fn client_icons(&self, client: &ClientInfo) -> Vec<u32> {
    let mut icons: Vec<u32> = Vec::new();
    let mut add = |id: u32| {
        if id != 0 && !icons.contains(&id) {
            icons.push(id);
        }
    };
    if let Some(group) = self.channel_groups.get(&client.channel_group) {
        add(group.icon);
    }
    let mut groups: Vec<&Group> = client.server_groups.iter().filter_map(|id| self.server_groups.get(id)).collect();
    groups.sort_by_key(|group| (group.sort, group.id));
    for group in groups {
        add(group.icon);
    }
    add(client.icon);
    icons
}
```

In `conn.rs` `on_command`, add arms that call these and set `self.view_dirty = true`: `notifyservergrouplist`, `notifychannelgrouplist`, `notifyservergroupclientadded`, `notifyservergroupclientdeleted`, `notifyclientchannelgroupchanged`. `notifyserveredited` already applies server fields.
- [x] **Step 4:** Run `cargo test -p ps-client`; expect all pass (12 tests).

### Task 2: Downloading an icon in `ps-client`

**Files:** Create `ps-client/src/filetransfer.rs`, `tools/seed_test_icons.py`; modify `ps-client/src/conn.rs`, `ps-client/src/lib.rs`, `ps-client/examples/probe.rs`.

**Interfaces:** Produces
`pub const ICON_SIZE_LIMIT: u64 = 512 * 1024;`
`pub fn init_download(transfer: u16, name: &str) -> Command`,
`pub struct Start { pub transfer: u16, pub server_transfer: u16, pub key: String, pub port: u16, pub size: u64 }`,
`pub fn parse_start(cmd: &Command) -> Option<Start>`,
`pub struct Status { pub transfer: u16, pub code: u32, pub message: String }`,
`pub fn parse_status(cmd: &Command) -> Option<Status>`,
`pub fn download(addr: SocketAddr, key: &str, size: u64, limit: u64, timeout: Duration) -> Result<Vec<u8>, String>`,
and on the public API: `ClientHandle::request_icon(&self, id: u32)`, `Event::Icon { id: u32, data: Result<Vec<u8>, String> }`, `ConnectOptions::filetransfer_port: Option<u16>` (a test hook like `simulated_loss`, `None` by default: when set, downloads go to this port instead of the one the server names).

- [x] **Step 1: Write the failing tests** in `filetransfer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve(payload: Vec<u8>, expect_key: &'static str) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut key = vec![0u8; expect_key.len()];
            stream.read_exact(&mut key).unwrap();
            assert_eq!(key, expect_key.as_bytes());
            stream.write_all(&payload).unwrap();
        });
        addr
    }

    #[test]
    fn builds_the_request_and_reads_the_replies() {
        assert_eq!(
            init_download(7, "/icon_2154984321").build(),
            "ftinitdownload clientftfid=7 name=\\/icon_2154984321 cid=0 cpw seekpos=0 proto=1"
        );
        let start = parse_start(&Command::parse(
            "notifystartdownload clientftfid=7 serverftfid=4 ftkey=7+MdQmmLmPE0b8XFx1z0pU9AGhuqZEEs port=30033 size=92 proto=1",
        ))
        .unwrap();
        assert_eq!((start.transfer, start.server_transfer, start.port, start.size), (7, 4, 30033, 92));
        assert_eq!(start.key, "7+MdQmmLmPE0b8XFx1z0pU9AGhuqZEEs");
        assert!(parse_start(&Command::parse("notifystartdownload clientftfid=7 port=30033 size=92")).is_none());
        let status = parse_status(&Command::parse(
            "notifystatusfiletransfer clientftfid=21 status=2054 msg=invalid\\sfile\\spath size=0",
        ))
        .unwrap();
        assert_eq!((status.transfer, status.code, status.message.as_str()), (21, 2054, "invalid file path"));
    }

    #[test]
    fn downloads_exactly_the_announced_bytes() {
        let payload: Vec<u8> = (0..92u8).collect();
        let addr = serve(payload.clone(), "0123456789abcdef0123456789abcdef");
        let got = download(addr, "0123456789abcdef0123456789abcdef", 92, ICON_SIZE_LIMIT, Duration::from_secs(2));
        assert_eq!(got, Ok(payload));
    }

    #[test]
    fn refuses_oversized_short_and_silent_transfers() {
        let unused: SocketAddr = "127.0.0.1:9".parse().unwrap();
        assert!(download(unused, "k", ICON_SIZE_LIMIT + 1, ICON_SIZE_LIMIT, Duration::from_secs(2)).is_err());

        let addr = serve(vec![1, 2, 3], "k");
        let short = download(addr, "k", 10, ICON_SIZE_LIMIT, Duration::from_secs(2));
        assert_eq!(short, Err("got 3 of 10 bytes".to_string()));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let keep = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(900));
            drop(stream);
        });
        let began = std::time::Instant::now();
        assert!(download(addr, "k", 10, ICON_SIZE_LIMIT, Duration::from_millis(300)).is_err());
        assert!(began.elapsed() < Duration::from_millis(800));
        keep.join().unwrap();
    }
}
```

- [x] **Step 2:** Run `cargo test -p ps-client filetransfer`; expect a compile failure.
- [x] **Step 3: Implement `filetransfer.rs`.** `init_download` builds the command in the order shown in the test, with an empty value for `cpw`. `parse_start` returns `None` unless `clientftfid`, `ftkey`, `port` and `size` are all present and the key is not empty. `parse_status` reads `clientftfid`, `status` and `msg`.

```rust
pub fn download(addr: SocketAddr, key: &str, size: u64, limit: u64, timeout: Duration) -> Result<Vec<u8>, String> {
    if size > limit {
        return Err(format!("the file is {size} bytes, more than the {limit} allowed"));
    }
    let mut stream =
        TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("cannot reach the file port: {e}"))?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.write_all(key.as_bytes()).map_err(|e| format!("cannot send the transfer key: {e}"))?;
    let began = Instant::now();
    let mut data = Vec::with_capacity(size as usize);
    let mut chunk = [0u8; 8192];
    while (data.len() as u64) < size {
        if began.elapsed() > timeout * 2 {
            return Err("the download took too long".to_string());
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let room = (size - data.len() as u64) as usize;
                data.extend_from_slice(&chunk[..n.min(room)]);
            }
            Err(e) => return Err(format!("the download stopped: {e}")),
        }
    }
    if data.len() as u64 != size {
        return Err(format!("got {} of {size} bytes", data.len()));
    }
    Ok(data)
}
```

- [x] **Step 4:** Run `cargo test -p ps-client filetransfer`; expect 3 pass.
- [x] **Step 5: Wire it into the connection.** In `conn.rs`:
  - `Request::Icon(u32)`; `ClientHandle::request_icon` sends it.
  - New `Conn` fields: `icon_queue: VecDeque<u32>`, `icon_asked: HashSet<u32>` (every id ever queued on this connection, so an id is requested once), `icon_active: Option<(u16, u32, Instant)>` (transfer id, icon id, when asked), `icon_job: Option<(u32, Receiver<Result<Vec<u8>, String>>)>`, `next_transfer: u16` (starts at 1), `next_icon_at: Instant`. The transfer thread gets its own channel; `Conn` must not hold a sender of its own request channel.
  - On `Request::Icon(id)`: ignore 0, the five standard ids and ids already in `icon_asked`; otherwise push to the queue. Ignore everything once the queue holds 600 ids.
  - In `tick`, when connected, nothing is active, no job is running, the queue is not empty and `now >= next_icon_at`: pop an id, send `init_download(transfer, "/icon_<id>")` with `send_tracked`, set `icon_active`, set `next_icon_at = now + 500 ms`.
  - `notifystartdownload` matching the active transfer: start a thread named `ps-client-ft` that calls `download(SocketAddr::new(<voice peer ip>, port), key, size, ICON_SIZE_LIMIT, 5 s)` and sends the result over the channel kept in `icon_job`. The port is `opts.filetransfer_port` when set, else the one in the reply. If `size` is over the limit, do not start a thread: send `ftstop serverftfid=<n> delete=0` and report the error.
  - `notifystatusfiletransfer` matching the active transfer: emit `Event::Icon { id, data: Err(message) }` and clear `icon_active`.
  - In `tick`, poll `icon_job` with `try_recv`; when it yields, emit `Event::Icon` and clear both. An active request with no answer after 10 s is reported as `Err("the server did not answer")` and cleared.
  - In `on_error` while connected: id `0x020c` (client is flooding) sets `next_icon_at = now + 15 s` and puts the active icon id back at the front of the queue.
- [x] **Step 6: Extend the probe.** `--icon ID` (repeatable) calls `request_icon` 700 ms after sign-in; each `Event::Icon` prints `[icon <id>] <n> bytes, starts <first 8 bytes in hex>` or `[icon <id>] failed: <reason>`. `--ft-port N` sets `filetransfer_port`.
- [x] **Step 7: Write `tools/seed_test_icons.py`.** It runs inside WSL against the test server's ServerQuery port. The upload and the three ways of assigning an icon below were tried by hand on 2026-10-06.

```python
import socket
import struct
import sys
import time
import zlib

COLOURS = [(240, 112, 95), (255, 192, 67), (30, 200, 120), (120, 150, 255), (200, 120, 220)]


def esc(text):
    return text.replace("\\", "\\\\").replace("/", "\\/").replace(" ", "\\s").replace("|", "\\p")


def unesc(text):
    return text.replace("\\s", " ").replace("\\p", "|").replace("\\/", "/").replace("\\\\", "\\")


def png(colour, salt):
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    raw = bytearray()
    for y in range(16):
        raw.append(0)
        for x in range(16):
            edge = x in (0, 15) or y in (0, 15)
            pixel = (255, 255, 255) if edge else colour
            if (x, y) == (1, 1):
                pixel = (salt & 255, (salt >> 8) & 255, 0)
            raw.extend(pixel)
    header = struct.pack(">IIBBBBB", 16, 16, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")


class Query:
    def __init__(self, password):
        self.sock = socket.create_connection(("127.0.0.1", 10011), timeout=5)
        self.buffer = b""
        self.read(b"specific command.")
        self.cmd(f"login serveradmin {password}")
        self.cmd("use sid=1")
        self.transfer = 0

    def read(self, marker):
        while marker not in self.buffer:
            chunk = self.sock.recv(65536)
            if not chunk:
                break
            self.buffer += chunk
        end = self.buffer.find(b"\n", self.buffer.find(marker))
        end = len(self.buffer) if end < 0 else end + 1
        out, self.buffer = self.buffer[:end], self.buffer[end:]
        return out.decode("utf-8", "replace")

    def cmd(self, text):
        self.sock.sendall(text.encode("utf-8") + b"\n")
        lines = [line.strip() for line in self.read(b"error id=").replace("\r", "").split("\n") if line.strip()]
        if "id=0 " not in lines[-1] + " ":
            print(f"  ! {text[:80]} -> {lines[-1]}")
        rows = []
        for line in lines[:-1]:
            for item in line.split("|"):
                rows.append({k: unesc(v) for k, _, v in (pair.partition("=") for pair in item.split(" "))})
        time.sleep(0.05)
        return rows

    def upload(self, name, data):
        self.transfer += 1
        rows = self.cmd(
            f"ftinitupload clientftfid={self.transfer} name={esc('/' + name)} cid=0 cpw= size={len(data)} overwrite=1 resume=0 proto=1"
        )
        if not rows or "ftkey" not in rows[0]:
            return False
        with socket.create_connection(("127.0.0.1", int(rows[0]["port"])), timeout=5) as link:
            link.sendall(rows[0]["ftkey"].encode("ascii"))
            link.sendall(data)
        time.sleep(0.2)
        return True


def icon(query, colour, salt):
    data = png(colour, salt)
    number = zlib.crc32(data) & 0xFFFFFFFF
    query.upload(f"icon_{number}", data)
    return number


def main():
    args = sys.argv[1:]
    query = Query(args[args.index("--password") + 1])
    if "--count" in args:
        for index in range(int(args[args.index("--count") + 1])):
            print(icon(query, COLOURS[index % len(COLOURS)], 1000 + index))
        return
    channels = {row["channel_name"]: row["cid"] for row in query.cmd("channellist")}
    guest = next(row["sgid"] for row in query.cmd("servergrouplist") if row["name"] == "Guest" and row.get("type") == "1")
    if "--clear" in args:
        for cid in channels.values():
            query.cmd(f"channeldelperm cid={cid} permsid=i_icon_id")
        query.cmd(f"servergroupdelperm sgid={guest} permsid=i_icon_id")
        query.cmd("serveredit virtualserver_icon_id=0")
        return
    first, second = icon(query, COLOURS[0], 1), icon(query, COLOURS[1], 2)
    query.upload("icon_600001", b"\x89PNG\r\n\x1a\n" + bytes(600 * 1024))
    query.upload("icon_600002", b"<html><body>not an icon</body></html>")
    query.cmd(f"channeladdperm cid={channels['Deep Rock']} permsid=i_icon_id permvalue={first}")
    query.cmd(f"channeladdperm cid={channels['Tide Pool']} permsid=i_icon_id permvalue={second}")
    query.cmd(f"channeladdperm cid={channels['Squad Alpha']} permsid=i_icon_id permvalue=600001")
    query.cmd(f"channeladdperm cid={channels['Radio']} permsid=i_icon_id permvalue=600002")
    query.cmd(f"servergroupaddperm sgid={guest} permsid=i_icon_id permvalue={second} permnegated=0 permskip=0")
    query.cmd(f"serveredit virtualserver_icon_id={first}")
    print(f"seeded: {first} on Deep Rock and the server, {second} on Tide Pool and Guest, two bad files")


main()
```

  The ServerQuery password is passed with `--password` and is never written into the file. If the server refuses the 600 KiB upload, note it in the ledger; the size limit is still covered by the unit tests.
- [x] **Step 8: Live check, happy path.** Start the test server, seed it, then
  `cargo run -p ps-client --example probe -- <server ip> --seconds 6 --icon <first> --icon <second> --icon 999`.
  Expected: two lines `92 bytes, starts 89504e470d0a1a0a` (the byte count may differ by a few), one `failed: invalid file path`, and at least 500 ms between requests in a `--log` run.
- [x] **Step 9: Live check, closed file port.** Same command with `--ft-port 9` added. Expected: each icon fails within 5 s with `cannot reach the file port`, the `[stats]` lines keep arriving every second, and the probe disconnects cleanly.
- [x] **Step 10: Live check, flood protection.** Put the defaults back through ServerQuery (`serveredit virtualserver_antiflood_points_tick_reduce=5 virtualserver_antiflood_points_needed_command_block=150 virtualserver_antiflood_points_needed_ip_block=250`), run the script with `--count 40`, and ask for all 40 ids with the probe. Expected: 40 icons received and no `[error 0x020c]` line. If the server complains, double the gap and repeat until it does not; put the gap that worked in the constant and in the ledger. Afterwards restore the relaxed values so other tests are not banned.

### Task 3: The icon store in `ps-app`

**Files:** Create `ps-app/src/icons.rs`; modify `ps-app/src/main.rs` (`mod icons;`).

**Interfaces:** Produces
`pub const MAX_ICON_BYTES: usize = 512 * 1024;`, `pub const MAX_ICON_SIDE: u32 = 256;`, `pub const RETRY_AFTER: Duration = Duration::from_secs(600);`,
`pub enum Format { Png, Jpeg, Gif, Bmp }`, `pub fn sniff(data: &[u8]) -> Option<(Format, u32, u32)>`,
`pub enum Reject { Empty, TooManyBytes, NotAnImage, Unsupported, TooLarge }`, `pub fn accept(data: &[u8]) -> Result<Format, Reject>`,
`pub fn server_folder(server_uid: &str) -> String`, `pub fn standard_icon(id: u32) -> Option<usize>` (index 0 to 4 for ids 100, 200, 300, 500, 600),
`pub enum Lookup { Standard(usize), Ready(slint::Image), Ask, Waiting, Nothing }`,
`pub struct IconStore` with `new(root: PathBuf) -> Self`, `lookup(&mut self, server_uid: &str, id: u32, now: Instant) -> Lookup`, `arrived(&mut self, server_uid: &str, id: u32, data: &[u8], now: Instant) -> Result<(), Reject>`, `failed(&mut self, server_uid: &str, id: u32, now: Instant)`.

- [x] **Step 1: Write the failing tests** in `icons.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        data
    }

    fn jpeg_header(width: u16, height: u16) -> Vec<u8> {
        let mut data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1]);
        data
    }

    #[test]
    fn reads_dimensions_from_headers() {
        assert_eq!(sniff(&png_header(16, 16)), Some((Format::Png, 16, 16)));
        assert_eq!(sniff(b"GIF89a\x10\x00\x20\x00\x00\x00\x00"), Some((Format::Gif, 16, 32)));
        assert_eq!(sniff(&jpeg_header(48, 24)), Some((Format::Jpeg, 48, 24)));
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 16]);
        bmp.extend_from_slice(&16i32.to_le_bytes());
        bmp.extend_from_slice(&(-16i32).to_le_bytes());
        assert_eq!(sniff(&bmp), Some((Format::Bmp, 16, 16)));
        assert_eq!(sniff(b"<html><body>404</body></html>"), None);
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None);
        assert_eq!(sniff(&png_header(16, 16)[..20]), None);
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF]), None);
        assert_eq!(sniff(&[]), None);
    }

    #[test]
    fn rejects_what_must_not_be_decoded() {
        assert_eq!(accept(&png_header(16, 16)), Ok(Format::Png));
        assert_eq!(accept(&jpeg_header(256, 256)), Ok(Format::Jpeg));
        assert_eq!(accept(&png_header(20000, 20000)), Err(Reject::TooLarge));
        assert_eq!(accept(&png_header(257, 16)), Err(Reject::TooLarge));
        assert_eq!(accept(&png_header(0, 16)), Err(Reject::TooLarge));
        assert_eq!(accept(b"GIF89a\x10\x00\x10\x00\x00\x00\x00"), Err(Reject::Unsupported));
        assert_eq!(accept(&[]), Err(Reject::Empty));
        assert_eq!(accept(b"not an image at all"), Err(Reject::NotAnImage));
        let mut big = png_header(16, 16);
        big.resize(MAX_ICON_BYTES + 1, 0);
        assert_eq!(accept(&big), Err(Reject::TooManyBytes));
    }

    #[test]
    fn each_server_gets_its_own_folder() {
        let a = server_folder("SwHpaSmzsKpQ4ksmdkMFOMpBhqA=");
        let b = server_folder("swhpasmzskpq4ksmdkmfompbhqa=");
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(server_folder("../../x"), "2e2e2f2e2e2f78");
        assert_eq!(server_folder(""), "unknown");
        assert_eq!(standard_icon(300), Some(2));
        assert_eq!(standard_icon(301), None);
    }

    #[test]
    fn an_icon_is_asked_for_once_and_retried_later() {
        let root = std::env::temp_dir().join(format!("parrotfish-icon-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut store = IconStore::new(root.clone());
        let start = Instant::now();
        assert!(matches!(store.lookup("serverA", 0, start), Lookup::Nothing));
        assert!(matches!(store.lookup("serverA", 600, start), Lookup::Standard(4)));
        assert!(matches!(store.lookup("serverA", 77, start), Lookup::Ask));
        assert!(matches!(store.lookup("serverA", 77, start), Lookup::Waiting));
        assert!(matches!(store.lookup("serverB", 77, start), Lookup::Ask));
        store.failed("serverA", 77, start);
        assert!(matches!(store.lookup("serverA", 77, start + Duration::from_secs(60)), Lookup::Nothing));
        assert!(matches!(store.lookup("serverA", 77, start + RETRY_AFTER + Duration::from_secs(1)), Lookup::Ask));
        assert_eq!(store.arrived("serverA", 77, b"junk", start), Err(Reject::NotAnImage));
        assert!(!root.join(server_folder("serverA")).join("icon_77").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
```

- [x] **Step 2:** Run `cargo test -p ps-app icons`; expect a compile failure.
- [x] **Step 3: Implement the checks.**

```rust
pub fn sniff(data: &[u8]) -> Option<(Format, u32, u32)> {
    if data.len() >= 24 && data.starts_with(b"\x89PNG\r\n\x1a\n") && &data[12..16] == b"IHDR" {
        let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        return Some((Format::Png, width, height));
    }
    if data.len() >= 10 && (data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) {
        let width = u32::from(u16::from_le_bytes([data[6], data[7]]));
        let height = u32::from(u16::from_le_bytes([data[8], data[9]]));
        return Some((Format::Gif, width, height));
    }
    if data.len() >= 26 && data.starts_with(b"BM") {
        let width = i32::from_le_bytes([data[18], data[19], data[20], data[21]]);
        let height = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
        return Some((Format::Bmp, width.unsigned_abs(), height.unsigned_abs()));
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        return jpeg_size(data).map(|(width, height)| (Format::Jpeg, width, height));
    }
    None
}

fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            return None;
        }
        let marker = data[at + 1];
        if marker == 0xFF {
            at += 1;
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            at += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([data[at + 2], data[at + 3]]));
        if length < 2 {
            return None;
        }
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            if at + 9 > data.len() {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([data[at + 5], data[at + 6]]));
            let width = u32::from(u16::from_be_bytes([data[at + 7], data[at + 8]]));
            return Some((width, height));
        }
        at += 2 + length;
    }
    None
}

pub fn accept(data: &[u8]) -> Result<Format, Reject> {
    if data.is_empty() {
        return Err(Reject::Empty);
    }
    if data.len() > MAX_ICON_BYTES {
        return Err(Reject::TooManyBytes);
    }
    let (format, width, height) = sniff(data).ok_or(Reject::NotAnImage)?;
    if width == 0 || height == 0 || width > MAX_ICON_SIDE || height > MAX_ICON_SIDE {
        return Err(Reject::TooLarge);
    }
    match format {
        Format::Png | Format::Jpeg => Ok(format),
        Format::Gif | Format::Bmp => Err(Reject::Unsupported),
    }
}

pub fn server_folder(server_uid: &str) -> String {
    if server_uid.is_empty() {
        return "unknown".to_string();
    }
    server_uid.bytes().map(|byte| format!("{byte:02x}")).collect()
}

pub fn standard_icon(id: u32) -> Option<usize> {
    [100, 200, 300, 500, 600].iter().position(|known| *known == id)
}
```

- [x] **Step 4: Implement `IconStore`.**

```rust
const REFRESH_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_ICONS_PER_SERVER: usize = 500;

enum Slot {
    Asked,
    Ready(slint::Image),
    Failed(Instant),
}

pub struct IconStore {
    root: PathBuf,
    slots: HashMap<(String, u32), Slot>,
}

impl IconStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root, slots: HashMap::new() }
    }

    fn file(&self, folder: &str, id: u32) -> PathBuf {
        self.root.join(folder).join(format!("icon_{id}"))
    }

    fn load(path: &Path) -> Option<slint::Image> {
        let age = std::fs::metadata(path).ok()?.modified().ok()?.elapsed().unwrap_or_default();
        if age > REFRESH_AFTER {
            return None;
        }
        let data = std::fs::read(path).ok()?;
        accept(&data).ok()?;
        slint::Image::load_from_path(path).ok()
    }

    pub fn lookup(&mut self, server_uid: &str, id: u32, now: Instant) -> Lookup {
        if id == 0 {
            return Lookup::Nothing;
        }
        if let Some(index) = standard_icon(id) {
            return Lookup::Standard(index);
        }
        let key = (server_folder(server_uid), id);
        match self.slots.get(&key) {
            Some(Slot::Ready(image)) => return Lookup::Ready(image.clone()),
            Some(Slot::Asked) => return Lookup::Waiting,
            Some(Slot::Failed(at)) if now.duration_since(*at) < RETRY_AFTER => return Lookup::Nothing,
            _ => {}
        }
        self.slots.remove(&key);
        if self.slots.keys().filter(|(folder, _)| *folder == key.0).count() >= MAX_ICONS_PER_SERVER {
            return Lookup::Nothing;
        }
        if let Some(image) = Self::load(&self.file(&key.0, id)) {
            self.slots.insert(key, Slot::Ready(image.clone()));
            return Lookup::Ready(image);
        }
        self.slots.insert(key, Slot::Asked);
        Lookup::Ask
    }

    pub fn arrived(&mut self, server_uid: &str, id: u32, data: &[u8], now: Instant) -> Result<(), Reject> {
        let key = (server_folder(server_uid), id);
        if let Err(reason) = accept(data) {
            self.slots.insert(key, Slot::Failed(now));
            return Err(reason);
        }
        let path = self.file(&key.0, id);
        let part = path.with_extension("part");
        let written = std::fs::create_dir_all(self.root.join(&key.0))
            .and_then(|_| std::fs::write(&part, data))
            .and_then(|_| std::fs::rename(&part, &path));
        let slot = match written.ok().and_then(|_| Self::load(&path)) {
            Some(image) => Slot::Ready(image),
            None => Slot::Failed(now),
        };
        self.slots.insert(key, slot);
        Ok(())
    }

    pub fn failed(&mut self, server_uid: &str, id: u32, now: Instant) {
        self.slots.insert((server_folder(server_uid), id), Slot::Failed(now));
    }
}
```

- [x] **Step 5:** Run `cargo test -p ps-app icons`; expect 4 pass.

### Task 4: Icons in the rows and the header

**Files:** Modify `ps-app/src/session.rs`, `ps-app/ui/widgets.slint`, `ps-app/ui/main.slint`, `ps-app/ui/theme.slint`; create `ps-app/ui/icons/group-100.svg`, `group-200.svg`, `group-300.svg`, `group-500.svg`, `group-600.svg`.

**Interfaces:** Consumes `ClientInfo::icons` and `Channel::icon` from Task 1. Produces `RowData::icons: Vec<u32>` (a channel row: its icon if any; a person row: at most four), and in Slint: `TreeRow.badges: int`, `TreeRow.badge-tint: int` (bit n set when slot n holds a standard icon), `TreeRow.badge-a`, `badge-b`, `badge-c`, `badge-d: image`; `Icons.group-100` to `Icons.group-600`; on `ParrotfishApp`: `in property <image> server-icon;` and `in property <bool> has-server-icon;`.

- [x] **Step 1: Write the failing test** by extending `rows_follow_the_tree` in `session.rs`: in `sample_view`, give the `Squad Alpha` channel `icon: 2154984321`, give `marlin` `icons: vec![100, 300, 452340182, 2154984321, 7]` and leave the others empty, then assert

```rust
assert_eq!(rows[6].icons, vec![2154984321]);
assert_eq!(rows[4].icons, vec![100, 300, 452340182, 2154984321]);
assert!(rows[0].icons.is_empty() && rows[1].icons.is_empty() && rows[2].icons.is_empty());
```

- [x] **Step 2:** Run `cargo test -p ps-app rows_follow`; expect a compile failure.
- [x] **Step 3:** Add `icons` to `RowData` and fill it in `build_rows` (channel: `channel.icon` when not 0; person: the first four of `client.icons`; spacers: none). Run the test; expect pass.
- [x] **Step 4: Draw the five standard icons** as 24 x 24 stroke-only white SVG files in the style of `ui/icons/*.svg`: 100 channel admin (a shield with a star), 200 operator (a wrench), 300 server admin (a shield with a tick), 500 query admin (a prompt, `>_`), 600 voice (a speech bubble). Add them to the `Icons` global in `theme.slint`.
- [x] **Step 5: Draw the slots.** In `widgets.slint` add

```slint
component Badge inherits Rectangle {
    in property <image> picture;
    in property <bool> tinted;
    width: 16px;
    height: 16px;

    if root.tinted: Image {
        width: 16px;
        height: 16px;
        source: root.picture;
        image-fit: contain;
        colorize: Theme.drift;
    }

    if !root.tinted: Image {
        width: 16px;
        height: 16px;
        source: root.picture;
        image-fit: contain;
    }
}
```

  and export it. In `TreeRowView`, in the channel row before the count and in the person row before the mute marks, add four slots of this shape, for `badge-a` to `badge-d` with thresholds 0 to 3 and tint bits 1, 2, 4, 8:

```slint
if root.entry.badges > 0: VerticalLayout {
    alignment: center;

    Badge {
        picture: root.entry.badge-a;
        tinted: Math.mod(root.entry.badge-tint, 2) >= 1;
    }
}
```

  (`Math.mod(tint / 2, 2) >= 1` for the second, `/ 4` and `/ 8` for the others, using `floor`.) An empty `image` draws nothing, which is how a slot is held open while its icon is on the way.
- [x] **Step 6:** In `main.slint`, after the server name in the header, inside a centred `VerticalLayout`: `if root.has-server-icon: Badge { picture: root.server-icon; }`.
- [x] **Step 7:** `cargo build -p ps-app`; expect no warnings.

### Task 5: Wiring, live checks and documents

**Files:** Modify `ps-app/src/app.rs`, `PLAN.md`, `README.md`.

**Interfaces:** Consumes everything above.

- [x] **Step 1: Wire the store.** `App` owns one `IconStore` rooted at `settings::config_dir().join("cache").join("icons")`. When rows are published for the viewed session, each id in `RowData::icons` and the server's icon go through `lookup(server.uid, id, now)`: `Ready` fills a slot; `Standard(n)` fills a slot from the `Icons` global and sets its tint bit; `Ask` calls `client.request_icon(id)` and holds the slot open; `Waiting` holds the slot open; `Nothing` takes no slot. `Event::Icon { id, data }` from a session calls `arrived` or `failed` with that session's server UID and marks the tree and the header dirty when that session is the viewed one. Sessions that are not viewed ask for nothing.
- [x] **Step 2:** `cargo test --workspace` and `cargo build --workspace --all-targets`; expect all tests pass (129) and no warnings.
- [x] **Step 3: Live check, what you see.** Seed the server, start the app with the software renderer and a throwaway profile (output volume 0), connect, and take screenshots. Expected: the coloured square on Deep Rock and Tide Pool, the Guest square on every person, the server icon after the server name, no icon and no error on Squad Alpha and Radio (the two bad files), row height unchanged at 26 px.
- [x] **Step 4: Live check, changes while connected.** With the app connected, through ServerQuery: add an icon to Lobby (`channeladdperm`), remove the one on Tide Pool (`channeldelperm cid=<id> permsid=i_icon_id`), add a connected test client to Server Admin (`servergroupaddclient sgid=<id> cldbid=<dbid>`). Expected in screenshots taken 2 s after each: Lobby gains the icon, Tide Pool loses it, the person gains the shield.
- [x] **Step 5: Live check, restart.** Close the app and start it again with `PARROTFISH_TRACE=1`, the server still up. Expected: the icons appear at once and the trace shows no `ftinitdownload` for icons already in the cache folder.
- [x] **Step 6: Live check, voice.** Connect twice (two identities, as in the compact-window test) with 40 icons seeded and a headless listener (`channeltest`) in the viewed channel. Expected: 50 voice packets per second throughout while the icons are fetched.
- [x] **Step 7: Documents.** In `PLAN.md` add an "Icons and file transfer" part to the protocol notes with the facts from this plan, and update the status, the crates table and the test count. In `README.md` add one line to the feature list. Run `build.bat`.

### Task 6 (only if you want GIF icons): first frame of a GIF

**Files:** Modify `ps-app/Cargo.toml`, `Cargo.toml` (workspace dependency `gif = "0.14"`, the version already in `Cargo.lock`), `ps-app/src/icons.rs`.

**Interfaces:** Changes `accept` to return `Ok(Format::Gif)`, and `IconStore::load` to decode GIF itself.

- [x] **Step 1: Change the test** `rejects_what_must_not_be_decoded`: the GIF line becomes `assert_eq!(accept(b"GIF89a\x10\x00\x10\x00\x00\x00\x00"), Ok(Format::Gif));`, and add `assert_eq!(accept(b"GIF89a\x01\x01\x10\x00\x00\x00\x00"), Err(Reject::TooLarge));` (257 wide). Add a test `decodes_the_first_gif_frame` that builds a 2 x 2 GIF with `gif::Encoder` in memory (four palette colours), passes it to `gif_first_frame`, and asserts width 2, height 2 and the four RGBA pixels.
- [x] **Step 2:** Run `cargo test -p ps-app icons`; expect failures.
- [x] **Step 3: Implement** `fn gif_first_frame(data: &[u8]) -> Option<(u32, u32, Vec<u8>)>` with `gif::DecodeOptions` set to `ColorOutput::RGBA` and a memory limit of 1 MiB, reading one frame and compositing it onto a transparent canvas of the logical screen size (already limited to 256 x 256 by `accept`). `IconStore::load` uses it for GIF and builds the image with `slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&pixels, width, height))`.
- [x] **Step 4:** Run `cargo test --workspace`; expect pass. Seed one GIF icon by hand through the script's `upload`, assign it to Lobby, and confirm it in a screenshot.

## Not covered, on purpose

Country flags, myTeamSpeak badges, avatars, group names beside nicknames, tooltips, animated GIFs, BMP and SVG icons, icon upload and management, a setting to turn icons off, and pruning the cache by total size.
