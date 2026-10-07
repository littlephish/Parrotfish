# Custom icons: what was done, decided and checked

Date: 2026-10-06. Plan: `2026-10-06-custom-icons.md`.

Tasks 1 to 5 were finished on 2026-10-06. Task 6 (GIF icons) was done on 2026-10-07, after you
asked whether TeamSpeak supports GIF and then for the remaining parity items: `gif` is now a
direct dependency of `ps-app`, and a GIF icon is shown as its first frame. This file records where the work
departed from the plan (each one is a "Ruling"), what you added while it was running, what was
checked and how, and what was not checked.

## Result

- 195 unit tests pass (`cargo test --workspace`): app 56, client 20, crypto 16, identity 14,
  protocol 34, audio 55. `cargo build --workspace --all-targets` has no warnings.
- Icons show on channels, on people (channel group, server groups, own icon) and after the
  server's name in the header and in the list of connected servers.
- Checked against the local TeamSpeak 3.13.8 server and against `ts.busaesi.space` (3.13.7),
  the server you named.

## Rulings

1. **Cache files carry an extension.** The plan stored `icon_<id>`. Slint decides a file's
   format from its name, so a file without an extension is never decoded. Files are
   `icon_<id>.png` or `icon_<id>.jpg`, named from the checked content, never from the server.
   A file whose name and content disagree is not loaded.
2. **One answer for every request.** The plan let the connection remember every id it was ever
   asked for and stay silent on repeats. Together with the store's retry after ten minutes that
   left an icon waiting for ever. The connection now forgets an id once it has answered, and
   answers a refused request (queue full, not connected) with an error.
3. **A session that ends gives its unanswered requests back.** Otherwise an icon asked for
   through a connection that then closed was never asked for again.
4. **An old icon stays on screen while a new copy is fetched.** The plan hid a file older than
   7 days until the download finished. If the download fails the old picture stays.
5. **A refused file is not fetched again in the same run.** The plan retried everything after
   ten minutes, which would fetch a GIF or a broken file again and again. Only a failed
   download is retried.
6. **Pictures larger than 32 x 32 are averaged down to 32 once.** Not in the plan. The software
   renderer scales by picking pixels, which made a 64 x 64 icon rough at 16 x 16.
   `ts.busaesi.space` uses one such icon.
7. **The file port is given up on after three failed tries, for ten minutes.** Not in the
   plan. On a hosted server with the port closed, every icon would otherwise cost a request and
   up to five seconds.
8. **Decoding is fenced off.** A panic inside the image decoder is caught and treated as a
   broken file, so a crafted file cannot close the program that way.
9. **At most 500 files per server on disk as well.** The plan limited only what is held in
   memory. The oldest files are removed when a new one is written.
10. **The gap between requests stays at 500 ms.** Measured as the plan asked: with the server's
    default flood protection, 110 requests sent at once were not blocked (of a burst of nickname
    changes the ninth was), and 40 icons at two a second drew no complaint.
11. **The server's icon is also shown in the list of connected servers**, after each name, at
    your request ("servers can also have custom icons"). The tiles keep their initials, as the
    plan decided.
12. **The probe gained `--all-icons`, `--save DIR` and `--voice`,** and `seed_test_icons.py`
    gained `--upload` and `--assign`, a 64 x 64 icon, a GIF and a 300 x 300 picture. The 600 KiB
    file could not be uploaded (the server dropped it); it was put into the server's folder by
    hand to check that it is refused before any byte is fetched.
13. **Not done: a tooltip or a setting.** As in the plan. There is no way yet to see which
    group an icon stands for, and no switch to turn icons off.

## What you added while this was running

- "servers can also have custom icons and we should use those as well, like
  ts.busaesi.space": Ruling 11, and that server was used as the real-world check.
- "server bookmarks can also have a setting of connect on startup" and "default channel
  password": built after the icons; see `PLAN.md`, "Known issues / decisions".
- "why am I hearing a tick/clicks/general noise when people stop speaking?": found and fixed
  in the player; see `PLAN.md`, "Voice" and "Verification done".

## Checked

- Unit tests for the id forms, group membership changes, the download (exact size, short,
  silent and trickling servers), the header checks, the per-server folders, asking once and
  retrying, keeping and finding files again, files that only look like images, and the limits.
- Live, test server: every kind of icon and every kind of refused file; changes while
  connected; restart from the cache with the files untouched; closed file port; default flood
  protection; voice complete while 40 icons were fetched.
- Live, `ts.busaesi.space`: one connection of about 25 seconds with a throwaway identity and
  the nickname PhishSpeak, and one of about 10 seconds from the test window. Five icons, all
  PNG, were fetched and drawn.

## Not checked

- The default renderer. Every screenshot used the software renderer.
- A server with hundreds of icons, or one that names another address for file transfer (the
  address in the answer is ignored on purpose).
- BMP and SVG icons are not shown at all. A GIF is shown as a still picture (its first frame);
  one was seeded on the test server and seen in the tree on 2026-10-07.
