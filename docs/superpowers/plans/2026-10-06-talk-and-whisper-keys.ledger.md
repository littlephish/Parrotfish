# Talk keys and whisper keys: what was done, decided and checked

Date: 2026-10-06. Plan: `2026-10-06-talk-and-whisper-keys.md`.

All nine tasks are finished except one step that only you can do (Task 9 Step 9, a whisper to and
from the official TeamSpeak client). Game controllers were left out at your request. This file
records the decisions I made without asking (each one is a "Ruling"), the things you added while
the work was running, what was checked and how, and what was not checked.

## Result

- 156 unit tests pass (`cargo test --workspace`): app 45, client 13, crypto 16, identity 14,
  protocol 34, audio 34. `cargo build --workspace --all-targets` has no warnings.
- Talk keys and whisper keys were driven against a TeamSpeak 3.13.8 server with keys pressed by
  program (F13 to F24) and headless listeners in six channels, and the windows were checked in
  software-rendered screenshots after real clicks.

## Rulings

Changes to how the plan was carried out:

1. Ruling: tests were written together with the code, not watched failing first. Every test the
   plan lists exists and passes.
2. Ruling: the shortcut code lives in `ps-app/src/app/shortcuts.rs`, a child module of `app.rs`.
   The plan named `app.rs` only; it was already 1400 lines.
3. Ruling: the plan did not say where the names of server and channel groups come from. The
   client now keeps the lists the server sends after login (`Event::Groups`), and the editor
   offers the regular groups (type 1) only.
4. Ruling: Task 9 Step 8 asked for a list of thirty channels of which twenty-nine do not exist.
   Channels that do not exist are dropped before anything is sent, so that would have tested a
   list of one. The check used twenty-nine real channels, made on the test server for the
   occasion and deleted afterwards.
5. Ruling: Task 9 Step 3 was done the way the plan allows for one server: the key's `server=`
   line in `whisper.ini` was changed to another UID.
6. Ruling: `channeltest` also got `--whisper` and `--commander`, so one tool can be the sender
   in the "being whispered to" and "channel commander" checks.

Behaviour the plan did not settle:

7. Ruling: four talk keys at most. The plan had no limit; the row has room for four.
8. Ruling: the key in a whisper key's row can be clicked and changed there, without opening the
   editor.
9. Ruling: a whisper key that names channels, people or a group and has no server recorded
   never sends (it could otherwise be read as "any server"). The limits of thirty channels and
   sixty people are applied again when the key is turned into a target, because one person can
   be connected several times.
10. Ruling: a list of channels and people belongs to one server. Open it while viewing another
    server and you see only what it holds, marked "not here now"; untick everything and it
    starts again on the server you are viewing. A group from another server is shown as
    "<name>, on <server>" and stays until you pick another.
11. Ruling: the reply key answers the last person who whispered to you on the server you are
    viewing. The target is checked by UID, so a client number the server has handed to someone
    else is not used, and it does not change while the reply key is held. If that person has
    gone, the dock says "The person who whispered to you has left".
12. Ruling: "Nobody is there to hear that whisper" stays for as long as the key is held and two
    seconds after, not just two seconds, because the server says it once per press. While it
    shows, the amber "sending" mark is off: nobody is hearing you.
13. Ruling: while a whisper is going out or cannot go out, the dock uses both of its lines for
    that sentence and hides your name. One line holds about 35 characters at the default width,
    and "Whispering to everyone, the channel above mine" was being cut off.
14. Ruling: where the stream to an audience stops for a reason the audio engine cannot see, the
    app sends the end marker itself, of the right kind: on mute, on a change of viewed server,
    and when whisper keys are edited while one is held. The frame destination is swapped while
    the transmitter is paused, so no frame can slip out between the two.
15. Ruling: turning "Let others whisper to me" off also unmarks anyone who is whispering to you
    at that moment.
16. Ruling: Esc closes the whisper key editor first and the settings window second, and does
    nothing to the window while a key is being chosen.
17. Ruling: the settings window opens 540 px high (it was 480) so the list of channels and
    people shows more rows, and a dropdown list opens upward when there is no room below it.
    Before, a long list near the bottom of a window was cut off.
18. Ruling: the tick list in the editor shows every channel and person of the server whatever is
    folded in the main window, and leaves out yourself, spacer channels and ServerQuery clients.

## Added by you while this was running

- "also allow for channels to be shrunk/expanded", "empty channels are contracted by default",
  "but make that an app setting", "Make all contracted / expanded a setting", "Remember my last
  client options for channels expanded or contracted individually". Built as: an arrow beside
  every channel that has people or channels inside it; Settings, Channels, "Channels start" with
  All open, Empty ones folded (the default) and All folded; what you fold or open by hand is
  saved per server and restored.
- "add a github ci build/tag step so installer + app is only fully built for release when we tag
  a release". Built as two workflows and a packaging script; see below.
- "Talking key should be 100% user assignable just not a pre selected enum". That is Part A of
  this plan: the list is gone and a key is chosen by pressing it.

Rulings inside those:

19. Ruling: "empty" means a channel that has channels inside it and nobody in any of them. A
    channel with people in it or below it is never folded by "Empty ones folded", so nobody is
    hidden by default.
20. Ruling: the way to your own channel is open when you arrive, in every mode, including after
    you had folded it by hand. You can fold it again; it then shows the count, stays highlighted
    and tints its icon while someone inside talks.
21. Ruling: changing "Channels start" forgets every fold you made by hand, on every server.
    Otherwise "All folded" would leave some channels open and look broken.
22. Ruling: the fold arrow is 20 px wide and as high as a row (26 px), below the 28 px minimum
    for controls, because tree rows are 26 px.
23. Ruling: remembered folds live in `settings.ini` (`folds.<server UID>=…`), at most 512
    channels a server and 64 servers; channels that no longer exist are dropped when saving.
24. Ruling: the release workflow runs the tests in release mode, links the C runtime into the
    program (in the workflow and in `tools/package_release.py` only; `build.bat` is unchanged),
    builds a per-user installer with Inno Setup that leaves `%APPDATA%\Parrotfish` alone, and
    refuses to run when the tag is not `v` plus the version in `Cargo.toml`. The installer is
    for 64-bit Intel/AMD Windows; the zip is the fallback elsewhere. Nothing is code-signed.

## Checked

Listed in `PLAN.md` under "Verification done": the talk key checks, every whisper check of
Task 9 Steps 1 to 8, the folding checks. In short: no whisper check delivered a single packet to
the app's own channel; every burst ended with exactly one end marker of its own kind; a key
pressed mid-sentence split the stream cleanly between the two audiences.

## Not checked

- Whispering to and from the official TeamSpeak client (Task 9 Step 9).
- A talk or whisper key held on a real keyboard or mouse; mouse buttons 3 to 5; keys while a
  game running as administrator has the focus.
- Sound by ear.
- The two workflows and the installer script. GitHub Actions cannot be run here and Inno Setup
  is not installed on this PC. What was checked: both files parse as YAML with the intended
  triggers, the packaging script's version check and its failure messages, and that the program
  builds and starts with the C runtime linked in.
