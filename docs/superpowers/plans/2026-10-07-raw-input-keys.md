# Raw Input Key Reading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a second way of reading the talk, whisper, reply and mute keys, in which Windows tells Parrotfish the moment a key moves (Windows calls this Raw Input, and it is how Mumble does it), and let the user switch between that and today's check in Settings.

**Architecture:** The part that decides what a set of held keys means (`WatchCore::step`) already takes "is this key down?" as a question it asks, so it does not change. Today a thread answers that question by asking Windows every 5 ms. The new way gives the same thread a hidden window that can never be focused, registers it with Windows to be told about every key and (only when needed) mouse button, keeps a small table of which chosen keys are down, and runs the same decision each time a key moves. The thread sleeps while nothing moves. A slow check against Windows' own key state guards against a microphone left open by a release that never arrived.

**Tech Stack:** Rust stable, Slint 1.18, existing workspace crates, more Win32 calls through the FFI blocks `platform.rs` already has. No new dependencies.

**Spec:** none. The next two sections are the scope and the decisions I made for you; change them before anyone starts.

**Status:** nothing is built in the repository. On 2026-10-07 the design was tried twice outside it: a standalone probe, and a throwaway copy of the workspace with Tasks 1 to 4 written out. What those showed is under "Facts".

**How to read the tasks:** tests, and the code where a mistake would leave the microphone open or the talk key dead, are given in full. Every Rust block in Tasks 1 to 4 except the one settings test was compiled in the throwaway copy, where the 17 tests they add passed together with the 81 the app has today, and a talk key was carried end to end with them. One thing differs from what was compiled: a field that recorded which way was in use, and the method that read it, are left out below because nothing needed them. The settings test in Task 4 and everything in Tasks 5 and 6 were written for this plan and have not been compiled. Steps that connect existing code are described by what they must do and are held to the tests and live checks listed with them.

## What this builds

One new row at the bottom of Settings, Shortcuts:

```
Shortcuts
  Talk keys          [ Left Ctrl  x ]  [ Mouse 4  x ]  [ Add a key ]
  After I let go     keep sending for  [----o-------]  0.2 s
  ...
  Incoming whispers  [x] Let others whisper to me

  Reading keys       [ Checked every 5 ms ][ As they happen ]
                     As they happen: Windows tells Parrotfish the moment a key moves, so it
                     reacts up to 5 ms sooner and rests while no key moves. Both ways work
                     while another program has the focus. If a key ever stops working,
                     switch back.
```

- **Checked every 5 ms** is what Parrotfish does today and stays the choice it starts with.
- **As they happen** is the new way. The switch takes effect at once; nothing has to be restarted and the keys you chose stay as they are.
- Both ways read the same keys: any key, mouse buttons 3 to 5, combinations of up to four. A settings file works with either.
- Both ways work with Parrotfish in the background, minimised, or never focused at all. That is the normal case, not a special one.
- If Windows refuses the new way, Parrotfish keeps checking every 5 ms and says so under the switch.

What you would notice with "As they happen": nothing audible. A press or release is acted on within a fraction of a millisecond where today it can take up to 5; a very short tap on a mute key cannot fall between two checks; and the key thread goes from about 200 wake-ups a second to one every 2 seconds while no key moves.

## How this compares with Mumble

| | Parrotfish today | Parrotfish with this plan | Mumble on Windows |
|---|---|---|---|
| How keys are read | asks Windows every 5 ms | told by Windows, or asks every 5 ms, your choice | told by Windows |
| Needs its window focused | no | no | no |
| Keyboard or mouse hook | none | none | none |
| Guard against a key that seems stuck | not needed, each check is fresh | compares with Windows every 50 ms while a key is held | I did not look for one |
| Mouse | buttons 3 to 5 | buttons 3 to 5, listened to only when one is chosen | always listened to |
| Game controllers | no | no (see "Not covered") | yes |
| Hides a key from other programs | no | no | no |

## Decisions I made that you have not confirmed

1. **The 5 ms check stays the default.** It is the way that has been used and tested, and "As they happen" is opt-in until it has had real use. Registering to be told about every key in the background is also the technique key-logging programs use, so a security or anti-cheat tool might look at it. Mumble, Discord and OBS do the same, and I have not checked how any such tool treats it.
2. **Wording.** The row is called "Reading keys" and the two choices are "Checked every 5 ms" and "As they happen". The words "Raw Input" do not appear in the window. Say so if you want them to.
3. **Same keys in both ways.** This plan does not add new kinds of keys. Keys that Windows gives no key code, more than five mouse buttons, and game controllers stay out. The new way is the one that could carry them later.
4. **A guard against an open microphone.** In the new way a key is "down" from the moment its press arrives until its release arrives. A release can fail to arrive: the PC is locked with Win+L while the key is held, a program running as administrator takes the focus, a keyboard is unplugged. So while any chosen key is believed held, Parrotfish also asks Windows about it every 50 ms, and lets it go after two answers of "not down" in a row. The worst case is a microphone open about 0.1 s longer than the key was held.
5. **Keys that guard cannot see are left alone.** If Windows' own key state never showed a press (a remapping tool can cause that), that press is released only by its own release. The other choice would cut such a key off after 0.1 s every time.
6. **Presses are never invented.** The guard only lets keys go. One consequence, on keyboards with an AltGr key: Windows tells the 5 ms check that AltGr is Left Ctrl plus Right Alt, and tells the new way only Right Alt. A talk key chosen as AltGr in one way has to be chosen again after switching. A talk key on Left Ctrl is no longer set off by typing with AltGr in the new way, which is a small improvement.
7. **The mouse is listened to only when it has to be:** while a chosen combination contains a mouse button, or while you are choosing a key. A gaming mouse can report thousands of movements a second and each one would wake Parrotfish for nothing. I could not measure that cost here (see "Facts").
8. **Keys nobody chose are dropped the moment they arrive.** In the new way every key press on the PC reaches Parrotfish. Only keys that are part of a chosen combination are remembered, as "down" or "not down", and nothing is ever written or logged. While you are choosing a key in Settings, every key counts until you have chosen.
9. **Switching is immediate and never leaves a key held.** The key thread is stopped and started again. If you are holding the talk key at that moment, your voice stops for an instant and continues. A key being chosen at that moment is abandoned.
10. **If Windows says no, fall back and say so.** No error window. The switch stays where you put it, a line under it reads "Windows did not allow that. Keys are still checked every 5 ms.", and the next start tries again.
11. **Parrotfish takes the registration from the window toolkit.** Windows allows one listener per kind of device in a program. The toolkit Parrotfish's windows are built with registers itself at start-up, for a feature Parrotfish does not use. The new way replaces that registration, and re-asserts its own every 2 seconds in case anything takes it back. See "Facts".

## Facts this plan rests on

Checked on 2026-10-07 on this PC unless marked "read".

**Parrotfish today** (read in the code):

- `keywatch.rs` runs one thread, `ps-keys`. Every 5 ms it calls `WatchCore::step(state, now, down)`, where `down` is a function "is this key code down?" answered with `GetAsyncKeyState`, and passes the result to the audio engine with `set_keys(talk, lane)`.
- `step` holds everything that matters: which talk, whisper and reply keys count as held (`evaluate`), the release delay (`Latch`), mute keys firing once per press (`Edges`), and choosing a key in Settings (`Capture`). It never asks how the answer to `down` was obtained. That is the seam this plan uses.
- The release delay needs the clock: after the last key is let go, `step` must be called again when the delay has passed. Today that happens by itself, 5 ms later.
- Keys are stored as Windows key codes, with left and right Ctrl, Shift and Alt told apart, and mouse buttons 3, 4 and 5 as codes 4, 5 and 6. `usable` refuses codes 0 to 2 (which include the left and right mouse buttons) and the three codes that do not say left or right.

**Windows** (read in Microsoft's reference pages for `GetAsyncKeyState`, `RAWINPUTDEVICE` and `RegisterRawInputDevices`):

- `RIDEV_INPUTSINK` "enables the caller to receive the input even when the caller is not in the foreground. Note that hwndTarget must be specified."
- "Only one window per raw input device class may be registered to receive raw input within a process (the window passed in the last call to RegisterRawInputDevices)."
- Removing a registration needs `RIDEV_REMOVE` with no window named, or the call fails.
- `GetAsyncKeyState` returns zero, meaning "not down", when "the current desktop is not the active desktop" (the lock screen and the administrator prompt are other desktops) and when "UI Privilege Isolation (UIPI) prevents the calling thread from accessing the foreground thread" (a program running as administrator has the focus). So today's check already goes blind in exactly the situations where the new way can lose a release, and the guard in Decision 4 lets a key go in those situations just as today's check does.

**The probe** (a standalone program outside the repository; it only reported F13 to F24 and counted everything else without recording it):

- A hidden message-only window registered with `RIDEV_INPUTSINK` received every key while the probe had no visible window and another program had the focus.
- Keys pressed by program with `keybd_event`, which is what `tools/hold_keys.py` does, arrive through Raw Input, marked as coming from no device. So the live checks can press F13 to F24 for the new way exactly as they do for the old.
- Windows' own key state agreed with every record at the moment it arrived: 8 out of 8.
- Raw Input was ahead of a 5 ms check running beside it by 3.9 and 4.0 ms on two presses and by 1.5 and 1.9 ms on two releases.
- Wake-ups: the 5 ms loop ran 1290 rounds in 7 s. The Raw Input loop woke 50 times in the same 7 s, nearly all of them for the probe's own timers and six at most for keys. Windows' counter of thread context switches showed about 860 a second for the 5 ms thread and about 10 for the other. That counter is the way to measure the same thing in the app: `Get-Counter '\Thread(ps-app*)\Context Switches/sec'` in PowerShell.
- Record sizes in a 64-bit build: header 24 bytes, keyboard part 16, mouse part 24, 48 in all.
- Registering the mouse as well succeeded. Nobody moved the mouse during that run, so the cost per movement was not measured.

**The throwaway copy of the workspace** (Tasks 1 to 4 as written below, then deleted):

- `cargo test -p ps-app`: 98 passed, which is today's 81 and 17 new. The build was clean apart from "never used" warnings for methods that only Task 5 calls.
- With `key_reading=raw`, a talk key of F24 held for 2 s gave a listener in the same channel 100 voice packets and one end-of-talk packet. The same with the 5 ms check: 100 and one.
- With a release delay of 300 ms: 115 and 116 packets in two runs. That is the timed wake-up for the release delay working.
- With the talk key set to F23+F24: 76 packets for 1.5 s of both keys, and nothing at all for F24 alone.
- In every one of those runs another program had the focus, checked before and after the keys were pressed.

**The window toolkit** (read in the sources this build uses, winit 0.30.13 and Slint's winit backend 1.18.1):

- winit registers every keyboard and mouse for Raw Input when it starts, with its own hidden window, so that programs can ask it for device events. It does this once, and again only if a program calls `listen_device_events`.
- Slint passes those device events to a hook that Parrotfish does not install. Nothing in Parrotfish depends on them.
- So turning on the new way takes the registration away from winit (see the one-window rule above) and turning it off removes it altogether. Neither has an effect anyone can see today. If a later version of the toolkit registered again while Parrotfish ran, the talk key would go dead in the new way; the 2-second renewal in Decision 11 is there for that.

**Mumble** (read, `src/mumble/GlobalShortcut_win.cpp` on its main branch):

- It calls `RegisterRawInputDevices` with `RIDEV_INPUTSINK` for keyboards, mice, joysticks, gamepads and multi-axis controllers, and handles `WM_INPUT` with `GetRawInputData`.
- It ignores the overrun make code and key code 0xFF, as Task 1 does.
- Xbox controllers and Logitech G-keys are polled. I found no code that hides a key from other programs.

**Not checked:**

- Mouse buttons through Raw Input. A mouse button cannot be pressed by program without the program that has the focus also receiving it (button 4 is "Back" in a browser), so the mouse path was checked only as far as "registering works". Its record parsing is unit-tested in Task 1.
- What happens at the lock screen, with a game in the focus, and with a program running as administrator in the focus. The first and third are reasoned from the documentation above.
- A keyboard with AltGr (Decision 6 is reasoned from how Windows reports that key).
- The cost of a mouse that reports thousands of movements a second.
- How anti-cheat and antivirus tools treat a program that listens to keys this way.

Pages read: learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getasynckeystate, .../ns-winuser-rawinputdevice, .../nf-winuser-registerrawinputdevices; github.com/mumble-voip/mumble `src/mumble/GlobalShortcut_win.cpp`.

## Global Constraints

- No new crates. Win32 calls live in `platform.rs` and nowhere else.
- **Parrotfish never has the focus when its keys matter.** Nothing in either way may depend on a Parrotfish window being focused, visible or even existing on screen. No live check counts unless another program had the focus while the keys were pressed; `tools/hold_keys.py --not-focused ps-app.exe` refuses to press otherwise.
- No keyboard or mouse hook in either way. No key is hidden from other programs.
- The 5 ms check stays, complete and unchanged in what it does, and is the default.
- The same stored keys work in both ways; nothing about a key is saved differently.
- In the new way a key that is not part of a chosen combination is dropped as it arrives. No key is ever written to disk or to a log, in either way.
- A release that never arrives may hold the microphone open for at most two guard checks (0.1 s).
- A whisper key must never cause ordinary channel voice to be sent, and nothing is sent while a key is being chosen. Both rules exist today and both ways go through the same `step`, so both keep them.
- Switching ways never leaves a key held and never fires a mute key.
- On systems other than Windows the new way does not exist: the code compiles, the listener cannot be opened, and the 5 ms loop runs.
- Wording as in "What this builds", sentence case. Controls at least 28 px high, with a hover state and a visible focus ring. Amber keeps its three meanings.
- No comments in code. Test scripts in Python. No real identities, UIDs or machine paths in tests, documents or scripts. Live checks use generated identities, a throwaway profile with output volume 0, and only the keys F13 to F24.
- Existing tests keep passing (256 today: app 81, client 32, crypto 16, identity 14, protocol 34, audio 79).

## Review Focus

Conditions the scope implies that are most likely to bite, each pinned to a task:

1. **The microphone stays open.** A release that never arrives: the PC locked while the talk key is held, an administrator's window taking the focus, a keyboard unplugged. (Task 1 tests `a_release_that_never_arrives_is_noticed`, `one_disagreement_is_forgiven`, `a_key_windows_cannot_see_is_held_until_its_own_release`; Task 6 check only you can do, with Win+L.)
2. **The talk key is dead.** Presses stop arriving because something else in the program registered for the same devices, or because the key was already held when the watcher started or the keys changed. (Task 1 test `keys_already_held_are_taken_over_when_asked`; the 2-second renewal in Task 4; Task 6 live check "held before the app started".)
3. **Focus.** Any step that only works because a Parrotfish window happened to be focused during a test. (The `--not-focused` rule on every live check in Task 6; the hidden window in Task 3 is message-only and cannot take the focus.)
4. **Timing that used to come for free.** The release delay, and the end of a chosen-key session, relied on a loop that came round every 5 ms. A loop that sleeps until a key moves has to wake itself for them. (Task 2 test `the_latch_says_when_a_held_over_key_runs_out`, Task 4 test `the_watcher_knows_when_a_held_over_key_runs_out`, Task 6 live check with a 300 ms delay.)
5. **Switching mid-press.** The switch flipped while a talk key or a mute key is held, or while a key is being chosen. (Task 4 test `a_watcher_that_starts_while_a_key_is_held_does_not_fire_it`; Task 6 live check "switching while held".)
6. **Several things in one wake-up.** A press and a release of a mute key arriving together must still count as one press. (Task 4 test `keys_reported_one_by_one_give_the_same_answers`.)
7. **Records that are not what they seem.** The made-up Shift around arrow keys, the two halves of Pause, left and right Ctrl, Shift and Alt, several mouse buttons in one record. (Task 1 tests on `key_from_raw` and `buttons_from_raw`.)
8. **Windows says no.** (Task 4: the thread falls back by itself; Task 5: the line under the switch.)

## File Structure

| File | Responsibility |
|---|---|
| `ps-app/src/rawkeys.rs` (new) | Turning Raw Input records into key codes; the table of chosen keys that are down, with the guard |
| `ps-app/src/hotkeys.rs` (modify) | Every key of every combination as one list; when a held-over key runs out |
| `ps-app/src/platform.rs` (modify) | The hidden window, the registration, reading records, waking the key thread |
| `ps-app/src/keywatch.rs` (modify) | The two ways as two loops around the same `step`; switching; falling back |
| `ps-app/src/settings.rs` (modify) | `key_reading` |
| `ps-app/src/app.rs`, `app/shortcuts.rs`, `main.rs` (modify) | Start with the saved way; the switch; the line when Windows refused |
| `ps-app/ui/settings.slint` (modify) | The "Reading keys" row |
| `tools/hold_keys.py` (modify) | `--not-focused`, so a check cannot pass by accident |
| `tools/hold_focus.py` (new) | A small window that takes the focus for a few seconds, for unattended checks |
| `PLAN.md`, `README.md` (modify) | How keys are read, the limits, the test count |

---

### Task 1: Raw records and the table of held keys

**Files:** Create `ps-app/src/rawkeys.rs`. Modify `ps-app/src/main.rs` (`mod rawkeys;` after `mod platform;`).

**Interfaces:** Produces
`pub fn key_from_raw(make: u16, flags: u16, vkey: u16) -> Option<(u16, bool)>` (key code and "is down"),
`pub fn buttons_from_raw(button_flags: u16) -> impl Iterator<Item = (u16, bool)>`,
`pub struct KeyTable` with `track(&mut self, keys: &[u16], everything: bool)`, `wants_mouse(&self) -> bool`, `set(&mut self, vk: u16, down: bool) -> bool` (true when that changed anything), `is_down(&self, vk: u16) -> bool`, `any_down(&self) -> bool`, `take_over(&mut self, windows_says_down: &dyn Fn(u16) -> bool) -> bool`, `release_lost(&mut self, windows_says_down: &dyn Fn(u16) -> bool) -> bool`.
Consumes `hotkeys::usable`.

- [ ] **Step 1: Write the failing tests** at the end of `rawkeys.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const MAKE: u16 = 0;
    const BREAK: u16 = 1;
    const E0: u16 = 2;
    const E1: u16 = 4;

    fn table(keys: &[u16]) -> KeyTable {
        let mut table = KeyTable::default();
        table.track(keys, false);
        table
    }

    #[test]
    fn raw_keys_get_the_codes_the_5_ms_check_uses() {
        assert_eq!(key_from_raw(0x76, MAKE, 0x87), Some((0x87, true)));
        assert_eq!(key_from_raw(0x76, BREAK, 0x87), Some((0x87, false)));
        assert_eq!(key_from_raw(0x1E, MAKE, 0x41), Some((0x41, true)));
        assert_eq!(key_from_raw(0x1D, MAKE, 0x11), Some((0xA2, true)), "left Ctrl");
        assert_eq!(key_from_raw(0x1D, E0, 0x11), Some((0xA3, true)), "right Ctrl");
        assert_eq!(key_from_raw(0x1D, E0 | BREAK, 0x11), Some((0xA3, false)));
        assert_eq!(key_from_raw(0x2A, MAKE, 0x10), Some((0xA0, true)), "left Shift");
        assert_eq!(key_from_raw(0x36, MAKE, 0x10), Some((0xA1, true)), "right Shift");
        assert_eq!(key_from_raw(0x38, MAKE, 0x12), Some((0xA4, true)), "left Alt");
        assert_eq!(key_from_raw(0x38, E0, 0x12), Some((0xA5, true)), "right Alt");
        assert_eq!(key_from_raw(0x1D, MAKE, 0xA2), Some((0xA2, true)), "a key already told apart stays as it is");
        assert_eq!(key_from_raw(0x1C, E0, 0x0D), Some((0x0D, true)), "Enter on the number pad is Enter");
        assert_eq!(key_from_raw(0x1D, E1, 0x13), Some((0x13, true)), "Pause");
    }

    #[test]
    fn records_that_are_not_keys_are_dropped() {
        assert_eq!(key_from_raw(0x2A, E0, 0xFF), None, "the made-up Shift around arrow keys");
        assert_eq!(key_from_raw(0x45, MAKE, 0xFF), None, "the second half of Pause");
        assert_eq!(key_from_raw(0xFF, MAKE, 0x41), None, "the keyboard reported an overrun");
        assert_eq!(key_from_raw(0x00, MAKE, 0x00), None);
        assert_eq!(key_from_raw(0x00, MAKE, 0x01), None, "mouse buttons never come from a keyboard");
        assert_eq!(key_from_raw(0x00, MAKE, 0x02), None);
        assert_eq!(key_from_raw(0x00, MAKE, 0x1FF), None);
    }

    #[test]
    fn mouse_records_give_buttons_three_to_five() {
        let all = |flags: u16| buttons_from_raw(flags).collect::<Vec<_>>();
        assert_eq!(all(0x0010), vec![(4, true)]);
        assert_eq!(all(0x0020), vec![(4, false)]);
        assert_eq!(all(0x0040), vec![(5, true)]);
        assert_eq!(all(0x0200), vec![(6, false)]);
        assert_eq!(all(0x0140), vec![(5, true), (6, true)], "one record can carry several buttons");
        assert_eq!(all(0x0001 | 0x0002 | 0x0004 | 0x0008), vec![], "left and right clicks are never talk keys");
        assert_eq!(all(0x0400), vec![], "the wheel is not a button");
        assert_eq!(all(0), vec![], "a record that is only movement");
    }

    #[test]
    fn a_key_is_down_from_its_press_to_its_release() {
        let mut keys = table(&[0x87, 0xA2]);
        assert!(!keys.any_down());
        assert!(keys.set(0x87, true));
        assert!(!keys.set(0x87, true), "a key that repeats while held changes nothing");
        assert!(keys.is_down(0x87) && !keys.is_down(0xA2) && keys.any_down());
        assert!(keys.set(0x87, false));
        assert!(!keys.set(0x87, false));
        assert!(!keys.is_down(0x87) && !keys.any_down());
        assert!(!keys.is_down(999));
    }

    #[test]
    fn keys_nobody_chose_are_not_kept() {
        let mut keys = table(&[0x87]);
        assert!(!keys.set(0x41, true));
        assert!(!keys.is_down(0x41) && !keys.any_down());
        assert!(!keys.set(0x10, true) && !keys.set(1, true) && !keys.set(0, true) && !keys.set(300, true));
        assert!(!keys.take_over(&|vk| vk == 0x41), "nor asked about");
        keys.track(&[0x87], true);
        assert!(keys.set(0x41, true), "while a key is being chosen every key counts");
        assert!(!keys.set(1, true), "but never the left mouse button");
        keys.track(&[0x87], false);
        assert!(!keys.is_down(0x41), "and is forgotten again afterwards");
        keys.set(0x87, true);
        keys.track(&[0x86], false);
        assert!(!keys.any_down(), "a key that is no longer chosen is let go");
    }

    #[test]
    fn the_mouse_is_only_listened_to_when_a_button_is_chosen() {
        assert!(!table(&[0x87, 0xA2]).wants_mouse());
        assert!(table(&[0x87, 5]).wants_mouse());
        assert!(table(&[4]).wants_mouse() && table(&[6]).wants_mouse());
        let mut keys = table(&[0x87]);
        keys.track(&[0x87], true);
        assert!(keys.wants_mouse(), "choosing a key may pick a mouse button");
    }

    #[test]
    fn a_release_that_never_arrives_is_noticed() {
        let mut keys = table(&[0x87]);
        keys.set(0x87, true);
        assert!(!keys.release_lost(&|_| true));
        assert!(!keys.release_lost(&|_| false), "one disagreement is not enough");
        assert!(keys.is_down(0x87));
        assert!(keys.release_lost(&|_| false));
        assert!(!keys.is_down(0x87));
        assert!(!keys.release_lost(&|_| false));
    }

    #[test]
    fn one_disagreement_is_forgiven() {
        let mut keys = table(&[0x87]);
        keys.set(0x87, true);
        keys.release_lost(&|_| true);
        keys.release_lost(&|_| false);
        keys.release_lost(&|_| true);
        assert!(!keys.release_lost(&|_| false), "the count starts again");
        assert!(keys.is_down(0x87));
    }

    #[test]
    fn a_key_windows_cannot_see_is_held_until_its_own_release() {
        let mut keys = table(&[0x87]);
        keys.set(0x87, true);
        for _ in 0..50 {
            assert!(!keys.release_lost(&|_| false));
        }
        assert!(keys.is_down(0x87));
        keys.set(0x87, false);
        keys.set(0x87, true);
        keys.release_lost(&|_| true);
        keys.set(0x87, false);
        keys.set(0x87, true);
        assert!(!keys.release_lost(&|_| false), "each press has to be seen by Windows afresh");
        assert!(!keys.release_lost(&|_| false));
        assert!(keys.is_down(0x87));
    }

    #[test]
    fn keys_already_held_are_taken_over_when_asked() {
        let mut keys = table(&[0x87, 0x86]);
        assert!(keys.take_over(&|vk| vk == 0x87 || vk == 0x41));
        assert!(keys.is_down(0x87), "held before the watcher started");
        assert!(!keys.is_down(0x86) && !keys.is_down(0x41));
        assert!(!keys.take_over(&|vk| vk == 0x87), "nothing new the second time");
        assert!(!keys.set(0x87, true), "its press arriving late changes nothing");
        assert!(!keys.release_lost(&|_| false));
        assert!(keys.release_lost(&|_| false), "and it can be let go like any other");
        assert!(!keys.any_down());
    }

    #[test]
    fn checking_for_lost_releases_never_presses_a_key() {
        let mut keys = table(&[0xA2, 0xA5]);
        keys.set(0xA5, true);
        for _ in 0..5 {
            keys.release_lost(&|vk| vk == 0xA2 || vk == 0xA5);
        }
        assert!(keys.is_down(0xA5));
        assert!(!keys.is_down(0xA2), "Windows reports Left Ctrl with AltGr; nobody pressed it");
    }
}
```

- [ ] **Step 2:** Run `cargo test -p ps-app rawkeys`; expect a compile failure.
- [ ] **Step 3: Implement** above the tests:

```rust
use crate::hotkeys::usable;

const KEYS: usize = 256;
const MISSES_BEFORE_RELEASE: u8 = 2;
const KEY_BREAK: u16 = 1;
const KEY_E0: u16 = 2;
const OVERRUN: u16 = 0xFF;
const LEFT_SHIFT_MAKE: u16 = 0x2A;

pub fn key_from_raw(make: u16, flags: u16, vkey: u16) -> Option<(u16, bool)> {
    if make == OVERRUN || vkey == 0 || vkey >= 0xFF {
        return None;
    }
    let extended = flags & KEY_E0 != 0;
    let vk = match vkey {
        0x10 if make == LEFT_SHIFT_MAKE => 0xA0,
        0x10 => 0xA1,
        0x11 if extended => 0xA3,
        0x11 => 0xA2,
        0x12 if extended => 0xA5,
        0x12 => 0xA4,
        other => other,
    };
    usable(vk).then_some((vk, flags & KEY_BREAK == 0))
}

pub fn buttons_from_raw(button_flags: u16) -> impl Iterator<Item = (u16, bool)> {
    const MOVES: [(u16, u16, bool); 6] =
        [(0x0010, 4, true), (0x0020, 4, false), (0x0040, 5, true), (0x0080, 5, false), (0x0100, 6, true), (0x0200, 6, false)];
    MOVES.into_iter().filter(move |(bit, _, _)| button_flags & bit != 0).map(|(_, vk, down)| (vk, down))
}

pub struct KeyTable {
    down: [bool; KEYS],
    confirmed: [bool; KEYS],
    misses: [u8; KEYS],
    tracked: [bool; KEYS],
    everything: bool,
}

impl Default for KeyTable {
    fn default() -> Self {
        Self { down: [false; KEYS], confirmed: [false; KEYS], misses: [0; KEYS], tracked: [false; KEYS], everything: false }
    }
}

impl KeyTable {
    fn follows(&self, vk: u16) -> bool {
        usable(vk) && (self.everything || self.tracked[usize::from(vk)])
    }

    fn forget(&mut self, index: usize) {
        self.down[index] = false;
        self.confirmed[index] = false;
        self.misses[index] = 0;
    }

    pub fn track(&mut self, keys: &[u16], everything: bool) {
        let mut tracked = [false; KEYS];
        for vk in keys.iter().copied().filter(|vk| usable(*vk)) {
            tracked[usize::from(vk)] = true;
        }
        self.tracked = tracked;
        self.everything = everything;
        for index in 0..KEYS {
            if self.down[index] && !self.follows(index as u16) {
                self.forget(index);
            }
        }
    }

    pub fn wants_mouse(&self) -> bool {
        self.everything || (4..=6).any(|vk| self.tracked[vk])
    }

    pub fn set(&mut self, vk: u16, down: bool) -> bool {
        if !self.follows(vk) {
            return false;
        }
        let index = usize::from(vk);
        let changed = self.down[index] != down;
        if changed {
            self.forget(index);
            self.down[index] = down;
        }
        changed
    }

    pub fn is_down(&self, vk: u16) -> bool {
        usize::from(vk) < KEYS && self.down[usize::from(vk)]
    }

    pub fn any_down(&self) -> bool {
        self.down.iter().any(|down| *down)
    }

    pub fn take_over(&mut self, windows_says_down: &dyn Fn(u16) -> bool) -> bool {
        let mut taken = false;
        for index in 0..KEYS {
            let vk = index as u16;
            if !self.down[index] && self.follows(vk) && windows_says_down(vk) {
                self.down[index] = true;
                self.confirmed[index] = true;
                self.misses[index] = 0;
                taken = true;
            }
        }
        taken
    }

    pub fn release_lost(&mut self, windows_says_down: &dyn Fn(u16) -> bool) -> bool {
        let mut released = false;
        for index in 0..KEYS {
            if !self.down[index] {
                continue;
            }
            if windows_says_down(index as u16) {
                self.confirmed[index] = true;
                self.misses[index] = 0;
            } else if self.confirmed[index] {
                self.misses[index] += 1;
                if self.misses[index] >= MISSES_BEFORE_RELEASE {
                    self.forget(index);
                    released = true;
                }
            }
        }
        released
    }
}
```

What the three flags on a key mean: `down` is what the records say; `confirmed` is set once Windows' own key state agreed during this press; `misses` counts answers of "not down" in a row since then. `set` clears the last two on every change, so each press earns its confirmation afresh.

- [ ] **Step 4:** Run `cargo test -p ps-app rawkeys`; expect 11 pass. The build warns that nothing uses the module yet; that goes away in Task 4.

### Task 2: What the watcher needs from the key rules

**Files:** Modify `ps-app/src/hotkeys.rs`.

**Interfaces:** Produces `Bindings::keys(&self) -> Vec<u16>` (every key of every combination, sorted, each once) and `Latch::due(&self) -> Option<Instant>` (when a key held over by the release delay runs out; `None` when nothing is held over).

- [ ] **Step 1: Write the failing tests** in the `tests` module of `hotkeys.rs`:

```rust
#[test]
fn every_key_of_every_combination_is_listed_once() {
    let bindings = Bindings {
        talk: vec![Chord::new(&[0xA2, 0x87]), Chord::new(&[0x05])],
        whisper: vec![Chord::new(&[0x86]), Chord::default()],
        reply: Chord::new(&[0xA2, 0x60]),
        actions: vec![Chord::new(&[0x7C]), Chord::default()],
    };
    assert_eq!(bindings.keys(), vec![0x05, 0x60, 0x7C, 0x86, 0x87, 0xA2]);
    assert!(Bindings::default().keys().is_empty());
}

#[test]
fn the_latch_says_when_a_held_over_key_runs_out() {
    let start = Instant::now();
    let delay = Duration::from_millis(300);
    let mut latch = Latch::default();
    assert_eq!(latch.due(), None);
    latch.update(start, Held { talk: true, lane: 2 }, delay);
    assert_eq!(latch.due(), None, "nothing runs out while the keys are held");
    latch.update(start + Duration::from_millis(100), Held { talk: true, lane: 0 }, delay);
    assert_eq!(latch.due(), Some(start + Duration::from_millis(400)));
    latch.update(start + Duration::from_millis(200), Held::default(), delay);
    assert_eq!(latch.due(), Some(start + Duration::from_millis(400)), "the earlier of the two");
    latch.update(start + Duration::from_millis(400), Held::default(), delay);
    assert_eq!(latch.due(), Some(start + Duration::from_millis(500)));
    assert_eq!(latch.update(start + Duration::from_millis(500), Held::default(), delay), Held::default());
    assert_eq!(latch.due(), None);
}
```

- [ ] **Step 2:** Run `cargo test -p ps-app hotkeys`; expect a compile failure.
- [ ] **Step 3: Implement.** After the `Bindings` struct:

```rust
impl Bindings {
    pub fn keys(&self) -> Vec<u16> {
        let chords = self.talk.iter().chain(&self.whisper).chain(&self.actions).chain(std::iter::once(&self.reply));
        let mut keys: Vec<u16> = chords.flat_map(|chord| chord.keys().iter().copied()).collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    }
}
```

and inside `impl Latch`, after `update`:

```rust
    pub fn due(&self) -> Option<Instant> {
        match (self.talk_until, self.lane_until) {
            (Some(talk), Some(lane)) => Some(talk.min(lane)),
            (talk, lane) => talk.or(lane),
        }
    }
```

- [ ] **Step 4:** Run `cargo test -p ps-app hotkeys`; expect all pass (2 new).

### Task 3: The listener

**Files:** Modify `ps-app/src/platform.rs`.

**Interfaces:** Produces, exported next to `key_down`:
`pub struct RawListener` with `open() -> Option<Self>`, `listen_to_mouse(&mut self, on: bool) -> bool`, `renew(&self) -> bool`, `wait(&self, timeout_ms: u32)`, `drain(&self, key: impl FnMut(u16, bool))`,
`pub fn this_thread() -> u32`, `pub fn wake_thread(thread: u32)`.
Consumes `rawkeys::key_from_raw` and `rawkeys::buttons_from_raw`.

A `RawListener` must be opened, used and dropped on one thread: Windows delivers a window's messages to the thread that created it.

There is nothing here a unit test can reach; every line talks to Windows. It is exercised by the live check at the end of Task 4, and it is the probe's code with the reporting taken out.

- [ ] **Step 1: Implement** inside the Windows `mod imp`, after `key_down`:

```rust
    #[repr(C)]
    struct RawDevice {
        usage_page: u16,
        usage: u16,
        flags: u32,
        target: isize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawHeader {
        kind: u32,
        size: u32,
        device: isize,
        wparam: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawKeyboard {
        make: u16,
        flags: u16,
        reserved: u16,
        vkey: u16,
        message: u32,
        extra: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RawMouse {
        flags: u16,
        pad: u16,
        button_flags: u16,
        button_data: u16,
        raw_buttons: u32,
        last_x: i32,
        last_y: i32,
        extra: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    union RawBody {
        mouse: RawMouse,
        keyboard: RawKeyboard,
    }

    #[repr(C)]
    struct RawRecord {
        header: RawHeader,
        body: RawBody,
    }

    #[repr(C)]
    struct Message {
        window: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterRawInputDevices(devices: *const RawDevice, count: u32, size: u32) -> i32;
        fn GetRawInputData(input: isize, command: u32, data: *mut core::ffi::c_void, size: *mut u32, header: u32) -> u32;
        fn CreateWindowExW(
            extended: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: isize,
            menu: isize,
            instance: isize,
            param: *const core::ffi::c_void,
        ) -> isize;
        fn DestroyWindow(window: isize) -> i32;
        fn MsgWaitForMultipleObjectsEx(count: u32, handles: *const isize, timeout: u32, wake: u32, flags: u32) -> u32;
        fn PeekMessageW(message: *mut Message, window: isize, first: u32, last: u32, remove: u32) -> i32;
        fn DispatchMessageW(message: *const Message) -> isize;
        fn PostThreadMessageW(thread: u32, message: u32, wparam: usize, lparam: isize) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    const MESSAGE_ONLY: isize = -3;
    const GENERIC_DESKTOP: u16 = 1;
    const USAGE_MOUSE: u16 = 2;
    const USAGE_KEYBOARD: u16 = 6;
    const IN_THE_BACKGROUND: u32 = 0x0000_0100;
    const STOP_LISTENING: u32 = 0x0000_0001;
    const READ_RECORD: u32 = 0x1000_0003;
    const KIND_MOUSE: u32 = 0;
    const KIND_KEYBOARD: u32 = 1;
    const ANY_INPUT: u32 = 0x04FF;
    const ALSO_WAITING_INPUT: u32 = 0x0004;
    const TAKE: u32 = 1;
    const WM_INPUT: u32 = 0x00FF;
    const WM_WAKE: u32 = 0x8000 + 0x51;

    pub struct RawListener {
        window: isize,
        mouse: bool,
    }

    impl RawListener {
        pub fn open() -> Option<Self> {
            let class: Vec<u16> = "Static".encode_utf16().chain(std::iter::once(0)).collect();
            let window = unsafe {
                CreateWindowExW(0, class.as_ptr(), std::ptr::null(), 0, 0, 0, 0, 0, MESSAGE_ONLY, 0, 0, std::ptr::null())
            };
            if window == 0 {
                return None;
            }
            let listener = Self { window, mouse: false };
            listener.register(USAGE_KEYBOARD, true).then_some(listener)
        }

        fn register(&self, usage: u16, on: bool) -> bool {
            let device = RawDevice {
                usage_page: GENERIC_DESKTOP,
                usage,
                flags: if on { IN_THE_BACKGROUND } else { STOP_LISTENING },
                target: if on { self.window } else { 0 },
            };
            unsafe { RegisterRawInputDevices(&device, 1, std::mem::size_of::<RawDevice>() as u32) != 0 }
        }

        pub fn listen_to_mouse(&mut self, on: bool) -> bool {
            if on != self.mouse && self.register(USAGE_MOUSE, on) {
                self.mouse = on;
            }
            self.mouse == on
        }

        pub fn renew(&self) -> bool {
            self.register(USAGE_KEYBOARD, true) && (!self.mouse || self.register(USAGE_MOUSE, true))
        }

        pub fn wait(&self, timeout_ms: u32) {
            unsafe { MsgWaitForMultipleObjectsEx(0, std::ptr::null(), timeout_ms, ANY_INPUT, ALSO_WAITING_INPUT) };
        }

        pub fn drain(&self, mut key: impl FnMut(u16, bool)) {
            let mut message: Message = unsafe { std::mem::zeroed() };
            while unsafe { PeekMessageW(&mut message, 0, 0, 0, TAKE) } != 0 {
                if message.message == WM_INPUT {
                    read_raw(message.lparam, &mut key);
                }
                unsafe { DispatchMessageW(&message) };
            }
        }
    }

    impl Drop for RawListener {
        fn drop(&mut self) {
            self.register(USAGE_KEYBOARD, false);
            if self.mouse {
                self.register(USAGE_MOUSE, false);
            }
            unsafe { DestroyWindow(self.window) };
        }
    }

    fn read_raw(handle: isize, key: &mut impl FnMut(u16, bool)) {
        let mut record: RawRecord = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<RawRecord>() as u32;
        let header = std::mem::size_of::<RawHeader>() as u32;
        let copied = unsafe { GetRawInputData(handle, READ_RECORD, (&mut record as *mut RawRecord).cast(), &mut size, header) };
        if copied == u32::MAX || copied < header {
            return;
        }
        match record.header.kind {
            KIND_KEYBOARD => {
                let keyboard = unsafe { record.body.keyboard };
                if let Some((vk, down)) = crate::rawkeys::key_from_raw(keyboard.make, keyboard.flags, keyboard.vkey) {
                    key(vk, down);
                }
            }
            KIND_MOUSE => {
                let mouse = unsafe { record.body.mouse };
                for (vk, down) in crate::rawkeys::buttons_from_raw(mouse.button_flags) {
                    key(vk, down);
                }
            }
            _ => {}
        }
    }

    pub fn this_thread() -> u32 {
        unsafe { GetCurrentThreadId() }
    }

    pub fn wake_thread(thread: u32) {
        if thread != 0 {
            unsafe { PostThreadMessageW(thread, WM_WAKE, 0, 0) };
        }
    }
```

Why it is shaped this way: the window's parent is `MESSAGE_ONLY`, which makes it a window that is never shown, never listed and cannot be focused; "Static" is a window class Windows already has, so none is registered. `IN_THE_BACKGROUND` is `RIDEV_INPUTSINK`. `wait` returns when a message is waiting or the time is up, whichever is first, and a message posted by `wake_thread` counts. `DispatchMessageW` lets Windows clean up each record after it has been read. A record that is neither keyboard nor mouse, or too large for the 48 bytes, is ignored.

- [ ] **Step 2: Implement the other systems** inside the non-Windows `mod imp`, after its `key_down`:

```rust
    pub struct RawListener;

    impl RawListener {
        pub fn open() -> Option<Self> {
            None
        }

        pub fn listen_to_mouse(&mut self, _on: bool) -> bool {
            false
        }

        pub fn renew(&self) -> bool {
            false
        }

        pub fn wait(&self, _timeout_ms: u32) {}

        pub fn drain(&self, _key: impl FnMut(u16, bool)) {}
    }

    pub fn this_thread() -> u32 {
        0
    }

    pub fn wake_thread(_thread: u32) {}
```

- [ ] **Step 3:** Add `this_thread`, `wake_thread` and `RawListener` to the `pub use imp::{...}` line. `cargo build -p ps-app`; expect only "never used" warnings for the new items.

### Task 4: Two ways of reading in the watcher, and the first live check

**Files:** Modify `ps-app/src/keywatch.rs`, `ps-app/src/settings.rs`, `ps-app/src/app.rs`.

**Interfaces:** Produces
`pub enum Reading { Checked, AsTheyHappen }` with `parse(&str) -> Self` and `to_text(self) -> &'static str` (`"poll"` and `"raw"`),
`WatchCore::fresh() -> Self`, `WatchCore::release_due(&self) -> Option<Instant>`,
`WatchState::refused(&self) -> bool`,
`KeyWatcher::start(audio: Arc<ps_voice::Shared>, reading: Reading) -> Self`, `KeyWatcher::set_reading(&mut self, reading: Reading)`,
`Settings::key_reading: Reading`.
`WatchCore::step` and everything else `WatchState` offers today keep their signatures.

- [ ] **Step 1: Write the failing tests.** In the `tests` module of `keywatch.rs`, with `use crate::rawkeys::KeyTable;` added to its imports:

```rust
#[test]
fn the_way_keys_are_read_is_kept_as_one_word() {
    assert_eq!(Reading::default(), Reading::Checked);
    assert_eq!(Reading::parse("raw"), Reading::AsTheyHappen);
    assert_eq!(Reading::parse(" raw "), Reading::AsTheyHappen);
    assert_eq!(Reading::parse("poll"), Reading::Checked);
    assert_eq!(Reading::parse(""), Reading::Checked);
    assert_eq!(Reading::parse("RAW input"), Reading::Checked, "anything unknown is the proven way");
    for reading in [Reading::Checked, Reading::AsTheyHappen] {
        assert_eq!(Reading::parse(reading.to_text()), reading);
    }
}

#[test]
fn a_watcher_that_starts_while_a_key_is_held_does_not_fire_it() {
    let state = WatchState::default();
    state.set_bindings(Bindings { actions: vec![Chord::new(&[0x88])], ..Bindings::default() });
    let mut core = WatchCore::fresh();
    let now = Instant::now();
    core.step(&state, now, &|vk| vk == 0x88);
    core.step(&state, now, &|vk| vk == 0x88);
    assert_eq!(state.take_fired(), 0, "held since before the watcher started");
    core.step(&state, now, &|_| false);
    core.step(&state, now, &|vk| vk == 0x88);
    assert_eq!(state.take_fired(), 1);
}

#[test]
fn the_watcher_knows_when_a_held_over_key_runs_out() {
    let state = WatchState::default();
    state.set_bindings(Bindings { talk: vec![Chord::new(&[0x87])], ..Bindings::default() });
    state.set_release_delay(300);
    let mut core = WatchCore::fresh();
    let start = Instant::now();
    assert_eq!(core.step(&state, start, &|vk| vk == 0x87), Held { talk: true, lane: 0 });
    assert_eq!(core.release_due(), None);
    let released = start + Duration::from_secs(1);
    assert_eq!(core.step(&state, released, &|_| false), Held { talk: true, lane: 0 });
    assert_eq!(core.release_due(), Some(released + Duration::from_millis(300)));
    assert_eq!(core.step(&state, released + Duration::from_millis(300), &|_| false), Held::default());
    assert_eq!(core.release_due(), None);
}

#[test]
fn keys_reported_one_by_one_give_the_same_answers() {
    let state = WatchState::default();
    state.set_bindings(Bindings {
        talk: vec![Chord::new(&[0xA2, 0x87])],
        whisper: vec![Chord::new(&[0x86])],
        reply: Chord::default(),
        actions: vec![Chord::new(&[0x88])],
    });
    assert_eq!(state.bound_keys(), vec![0x86, 0x87, 0x88, 0xA2]);
    let mut keys = KeyTable::default();
    keys.track(&state.bound_keys(), false);
    let mut core = WatchCore::fresh();
    let now = Instant::now();
    let mut feed = |keys: &mut KeyTable, vk: u16, down: bool| {
        keys.set(vk, down);
        core.step(&state, now, &|vk| keys.is_down(vk))
    };
    assert_eq!(feed(&mut keys, 0x87, true), Held::default(), "half a combination");
    assert_eq!(feed(&mut keys, 0x41, true), Held::default(), "a key nobody chose");
    assert_eq!(feed(&mut keys, 0xA2, true), Held { talk: true, lane: 0 });
    assert_eq!(feed(&mut keys, 0x86, true), Held { talk: true, lane: 1 });
    assert_eq!(feed(&mut keys, 0x87, false), Held { talk: false, lane: 1 });
    assert_eq!(feed(&mut keys, 0x86, false), Held::default());
    assert_eq!(state.take_fired(), 0);
    feed(&mut keys, 0x88, true);
    feed(&mut keys, 0x88, false);
    assert_eq!(state.take_fired(), 1, "a tap is one press however short it was");
}
```

In the `tests` module of `settings.rs` (this one test was not compiled for the plan):

```rust
#[test]
fn the_way_keys_are_read_is_remembered() {
    assert_eq!(Settings::default().key_reading, Reading::Checked);
    let mut s = Settings::default();
    s.key_reading = Reading::AsTheyHappen;
    assert!(s.serialize().contains("key_reading=raw\n"));
    assert_eq!(Settings::parse(&s.serialize()), s);
    assert_eq!(Settings::parse("key_reading=poll\n").key_reading, Reading::Checked);
    assert_eq!(Settings::parse("key_reading=fastest\n").key_reading, Reading::Checked);
    assert_eq!(Settings::parse("nickname=Minnow\n").key_reading, Reading::Checked, "a file from before this setting");
}
```

- [ ] **Step 2:** Run `cargo test -p ps-app keywatch` and `cargo test -p ps-app settings`; expect compile failures.
- [ ] **Step 3: Implement in `keywatch.rs`.** The imports, constants and the new type:

```rust
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::hotkeys::{evaluate, usable, Bindings, Capture, CaptureStep, Edges, Held, Latch};
use crate::platform::{self, RawListener};
use crate::rawkeys::KeyTable;

const POLL_EVERY: Duration = Duration::from_millis(5);
const CHECK_WHILE_HELD: Duration = Duration::from_millis(50);
const RENEW_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reading {
    #[default]
    Checked,
    AsTheyHappen,
}

impl Reading {
    pub fn parse(text: &str) -> Self {
        if text.trim() == "raw" {
            Self::AsTheyHappen
        } else {
            Self::Checked
        }
    }

    pub fn to_text(self) -> &'static str {
        match self {
            Self::Checked => "poll",
            Self::AsTheyHappen => "raw",
        }
    }
}
```

`WatchState` gains three fields, and `WatchCore` two functions in front of `step`, which is not touched:

```rust
#[derive(Default)]
pub struct WatchState {
    bindings: Mutex<Bindings>,
    version: AtomicU64,
    delay_ms: AtomicU32,
    capturing: AtomicBool,
    captured: Mutex<Option<CaptureStep>>,
    fired: AtomicU32,
    stop: AtomicBool,
    raw_thread: AtomicU32,
    refused: AtomicBool,
}

impl WatchCore {
    pub fn fresh() -> Self {
        let mut core = Self::default();
        core.edges.block();
        core
    }

    pub fn release_due(&self) -> Option<Instant> {
        self.latch.due()
    }
}
```

`impl WatchState` becomes the following. `set_bindings`, `begin_capture` and `cancel_capture` now also wake the key thread, because in the new way it may be asleep with no reason to look:

```rust
impl WatchState {
    fn wake(&self) {
        platform::wake_thread(self.raw_thread.load(Ordering::Relaxed));
    }

    fn bound_keys(&self) -> Vec<u16> {
        self.bindings.lock().map(|bindings| bindings.keys()).unwrap_or_default()
    }

    pub fn set_bindings(&self, bindings: Bindings) {
        if let Ok(mut slot) = self.bindings.lock() {
            *slot = bindings;
        }
        self.version.fetch_add(1, Ordering::Relaxed);
        self.wake();
    }

    pub fn set_release_delay(&self, ms: u32) {
        self.delay_ms.store(ms.min(1000), Ordering::Relaxed);
    }

    pub fn begin_capture(&self) {
        if let Ok(mut slot) = self.captured.lock() {
            *slot = None;
        }
        self.capturing.store(true, Ordering::Relaxed);
        self.wake();
    }

    pub fn cancel_capture(&self) {
        self.capturing.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.captured.lock() {
            *slot = None;
        }
        self.wake();
    }

    pub fn take_fired(&self) -> u32 {
        self.fired.swap(0, Ordering::Relaxed)
    }

    pub fn take_captured(&self) -> Option<CaptureStep> {
        self.captured.lock().ok().and_then(|mut slot| slot.take())
    }

    pub fn refused(&self) -> bool {
        self.refused.load(Ordering::Relaxed)
    }
}
```

The two loops and the watcher replace everything from `pub struct KeyWatcher` to the end of its `Drop`:

```rust
fn windows_says_down(vk: u16) -> bool {
    platform::key_down(i32::from(vk))
}

fn watch_checked(shared: &WatchState, audio: &ps_voice::Shared) {
    let mut core = WatchCore::fresh();
    while !shared.stop.load(Ordering::Relaxed) {
        let held = core.step(shared, Instant::now(), &windows_says_down);
        audio.set_keys(held.talk, held.lane);
        std::thread::sleep(POLL_EVERY);
    }
}

fn watch_as_they_happen(shared: &WatchState, audio: &ps_voice::Shared, mut listener: RawListener) {
    let mut core = WatchCore::fresh();
    let mut keys = KeyTable::default();
    let mut followed: Option<(u64, bool)> = None;
    let mut next_check = Instant::now();
    let mut next_renewal = Instant::now() + RENEW_EVERY;
    shared.raw_thread.store(platform::this_thread(), Ordering::Relaxed);
    while !shared.stop.load(Ordering::Relaxed) {
        let wanted = (shared.version.load(Ordering::Relaxed), shared.capturing.load(Ordering::Relaxed));
        if followed != Some(wanted) {
            keys.track(&shared.bound_keys(), wanted.1);
            keys.take_over(&windows_says_down);
            listener.listen_to_mouse(keys.wants_mouse());
            followed = Some(wanted);
        }
        listener.drain(|vk, down| {
            if keys.set(vk, down) {
                let held = core.step(shared, Instant::now(), &|vk| keys.is_down(vk));
                audio.set_keys(held.talk, held.lane);
            }
        });
        let now = Instant::now();
        if keys.any_down() && now >= next_check {
            keys.release_lost(&windows_says_down);
            next_check = now + CHECK_WHILE_HELD;
        }
        if now >= next_renewal {
            listener.renew();
            next_renewal = now + RENEW_EVERY;
        }
        let held = core.step(shared, now, &|vk| keys.is_down(vk));
        audio.set_keys(held.talk, held.lane);
        let mut sleep = next_renewal.saturating_duration_since(now);
        if keys.any_down() {
            sleep = sleep.min(next_check.saturating_duration_since(now));
        }
        if let Some(due) = core.release_due() {
            sleep = sleep.min(due.saturating_duration_since(now));
        }
        listener.wait(sleep.as_micros().div_ceil(1000) as u32);
    }
    shared.raw_thread.store(0, Ordering::Relaxed);
}

pub struct KeyWatcher {
    state: Arc<WatchState>,
    audio: Arc<ps_voice::Shared>,
    thread: Option<JoinHandle<()>>,
    wanted: Reading,
}

impl KeyWatcher {
    pub fn start(audio: Arc<ps_voice::Shared>, reading: Reading) -> Self {
        let mut watcher = Self { state: Arc::new(WatchState::default()), audio, thread: None, wanted: reading };
        watcher.spawn();
        watcher
    }

    fn spawn(&mut self) {
        let shared = self.state.clone();
        let audio = self.audio.clone();
        let reading = self.wanted;
        shared.stop.store(false, Ordering::Relaxed);
        shared.refused.store(false, Ordering::Relaxed);
        let thread = std::thread::Builder::new()
            .name("ps-keys".into())
            .spawn(move || {
                let listener = if reading == Reading::AsTheyHappen { RawListener::open() } else { None };
                match listener {
                    Some(listener) => watch_as_they_happen(&shared, &audio, listener),
                    None => {
                        shared.refused.store(reading == Reading::AsTheyHappen, Ordering::Relaxed);
                        watch_checked(&shared, &audio);
                    }
                }
                audio.set_keys(false, 0);
            })
            .expect("failed to spawn the key watcher thread");
        self.thread = Some(thread);
    }

    fn halt(&mut self) {
        self.state.stop.store(true, Ordering::Relaxed);
        self.state.wake();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }

    pub fn set_reading(&mut self, reading: Reading) {
        if reading != self.wanted {
            self.halt();
            self.wanted = reading;
            self.spawn();
        }
    }

    pub fn state(&self) -> &WatchState {
        &self.state
    }
}

impl Drop for KeyWatcher {
    fn drop(&mut self) {
        self.halt();
    }
}
```

How the new loop meets the Review Focus, line by line:

- **Each record gets its own decision.** `step` runs inside `drain` for every change, so a press and a release that arrive together are two decisions, and a tap is one press (focus 6).
- **It wakes itself only when time matters:** for the guard while a key is held (50 ms), for the release delay (`release_due`), and every 2 s to renew the registration. With no key held and no delay pending it sleeps until a key moves, something wakes it, or the 2 s are up (focus 4).
- **`take_over` runs whenever the set of followed keys changes,** which includes the first time round: a key already held when the watcher starts, when a key has just been chosen, or when the switch is flipped is picked up at once (focus 2, 5).
- **`WatchCore::fresh` starts with mute keys blocked** until they have all been seen up, so a mute key held across a switch does not fire (focus 5). The 5 ms loop uses it too; today a mute key held while Parrotfish starts fires once, and after this it does not.
- **Stopping:** `halt` sets the flag and posts a wake-up. If the thread has not stored its id yet, the wake-up goes nowhere and the loop sees the flag before its first sleep. If it is about to sleep, the posted message ends the sleep at once.
- **The listener is opened on the key thread** and dropped there when the loop returns, as Task 3 requires.

- [ ] **Step 4: Implement in `settings.rs` and `app.rs`.** `Settings` gets `pub key_reading: Reading` (default `Reading::Checked`) after `mute_sound_key`; `parse` reads `"key_reading" => s.key_reading = Reading::parse(value)`; `serialize` writes `put("key_reading", self.key_reading.to_text().to_string())` after the mute keys. In `App::new`, `KeyWatcher::start(engine.shared().clone(), settings.key_reading)`.
- [ ] **Step 5:** Run `cargo test --workspace`; expect all pass (274: 18 new). `cargo build --workspace --all-targets`; expect warnings only for `refused` and `set_reading`, which Task 5 uses.
- [ ] **Step 6: Extend `tools/hold_keys.py`** so that a check cannot pass because Parrotfish happened to be focused (tried on 2026-10-07: exit codes 0, 2 and 3 as written):

```python
import ctypes
import ctypes.wintypes as wt
import sys
import time

ALLOWED = {f"F{number}": 0x7C + number - 13 for number in range(13, 25)}
USAGE = "usage: hold_keys.py F13..F24[+F13..F24] <seconds> [--not-focused program.exe]"

user32 = ctypes.WinDLL("user32")
kernel32 = ctypes.WinDLL("kernel32")
user32.GetForegroundWindow.restype = wt.HWND
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
kernel32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
kernel32.OpenProcess.restype = wt.HANDLE
kernel32.QueryFullProcessImageNameW.argtypes = [wt.HANDLE, wt.DWORD, wt.LPWSTR, ctypes.POINTER(wt.DWORD)]
kernel32.CloseHandle.argtypes = [wt.HANDLE]


def focused_program():
    window = user32.GetForegroundWindow()
    if not window:
        return ""
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(window, ctypes.byref(pid))
    handle = kernel32.OpenProcess(0x1000, False, pid.value)
    if not handle:
        return ""
    buffer = ctypes.create_unicode_buffer(1024)
    size = wt.DWORD(1024)
    found = kernel32.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(size))
    kernel32.CloseHandle(handle)
    return buffer.value.rsplit("\\", 1)[-1].lower() if found else ""


def main():
    args = sys.argv[1:]
    unfocused = ""
    if "--not-focused" in args:
        at = args.index("--not-focused")
        if at + 1 >= len(args):
            print(USAGE)
            sys.exit(2)
        unfocused = args[at + 1].lower()
        del args[at:at + 2]
    if len(args) != 2 or any(name not in ALLOWED for name in args[0].split("+")):
        print(USAGE)
        sys.exit(2)
    if unfocused and focused_program() == unfocused:
        print(f"{unfocused} has the focus; this check needs another program to have it")
        sys.exit(3)
    codes = [ALLOWED[name] for name in args[0].split("+")]
    for code in codes:
        user32.keybd_event(code, 0, 0, 0)
    time.sleep(float(args[1]))
    for code in reversed(codes):
        user32.keybd_event(code, 0, 2, 0)
    if unfocused and focused_program() == unfocused:
        print(f"{unfocused} took the focus while the keys were held; this check proves nothing")
        sys.exit(3)
    where = f", while {unfocused} did not have the focus" if unfocused else ""
    print(f"held {args[0]} for {args[1]} s{where}")


main()
```

and **write `tools/hold_focus.py`**, for when nobody is at the PC to click another window (tried the same day; the script above then reports `python.exe` as focused):

```python
import sys
import tkinter

seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 8.0
root = tkinter.Tk()
root.title("focus holder for a Parrotfish test")
root.geometry("260x60+60+60")
tkinter.Label(root, text="Holding the focus for a key test.\nCloses by itself.").pack(expand=True)
root.lift()
root.focus_force()
root.after(int(seconds * 1000), root.destroy)
root.mainloop()
```

- [ ] **Step 7: Live check, the new way carries a talk key.** Throwaway profile with `tx_mode=1`, `talk_key=135` (F24), `key_reading=raw`, output volume 0. Test server up, `cargo run -p ps-voice --example channeltest -- <server ip> --join <Lobby id> --nick Listener --seconds 9` listening, the app connected to the same channel. If Parrotfish has the focus, run `python tools/hold_focus.py 6` first. Then `python tools/hold_keys.py F24 2 --not-focused ps-app.exe`. Expected from the listener: a first voice packet, about 100 packets, one end-of-talk packet; and from the tool, "held F24 for 2 s, while ps-app.exe did not have the focus". Repeat with `key_reading=poll`: the same numbers.

### Task 5: The switch in Settings

**Files:** Modify `ps-app/ui/settings.slint`, `ps-app/src/app/shortcuts.rs`, `ps-app/src/app.rs`, `ps-app/src/main.rs`.

**Interfaces:** Consumes `KeyWatcher::set_reading`, `WatchState::refused`, `Settings::key_reading`. Produces in `SettingsWindow`: `in-out property <int> key-reading` (0 or 1), `in property <bool> key-reading-refused`, `callback key-reading-changed()`.

- [ ] **Step 1: The row.** In the Shortcuts tab, after the "Incoming whispers" row and a `Divider`: a `Caption` "Reading keys" and, beside it, a column with a `Segmented` whose options are `["Checked every 5 ms", "As they happen"]`, bound to `key-reading` and calling `key-reading-changed()` when a choice is made (the same shape as "Channels start" on the Channels tab); a `Hint` with the text from "What this builds"; and, only while `key-reading-refused`, a `Hint` in `Theme.coral` reading "Windows did not allow that. Keys are still checked every 5 ms."
- [ ] **Step 2: The handler.** `App::key_reading_changed` in `shortcuts.rs` reads the property, and when the choice differs from `settings.key_reading`: cancels a key that is being chosen (the same call closing the settings window makes), stores the choice, calls `self.watcher.set_reading(...)`, and marks the settings to be saved. The keys are not sent again; the watcher keeps them across the restart. `publish_shortcuts` sets `key-reading` from the setting. `main.rs` connects the callback like its neighbours.
- [ ] **Step 3: The line when Windows refused.** The key thread decides a moment after `set_reading` returns, so the app does not ask once: the 33 ms tick compares `self.watcher.state().refused()` with what it last showed and sets `key-reading-refused` when that changes.
- [ ] **Step 4:** `cargo build --workspace --all-targets`; expect no warnings. `cargo test --workspace`; expect 274.
- [ ] **Step 5: Screenshots** with the software renderer of the Shortcuts tab scrolled to the end, in both positions of the switch. Check: sentence case, the control 28 px high or more, the hint not cut off at the window's smallest size, no second amber button.

### Task 6: Live checks and documents

**Files:** Modify `PLAN.md`, `README.md`. The checks use `tools/hold_keys.py`, `tools/hold_focus.py`, `channeltest` and the test server; nothing else is written.

Every check below is run with `key_reading=raw` unless it says otherwise, with the throwaway profile and output volume 0, and every `hold_keys.py` call carries `--not-focused ps-app.exe`. A check where that tool exits with 3 is run again, not counted.

- [ ] **Step 1: The release delay.** `talk_release_ms=300`, talk key F24, a listener in the channel. `hold_keys.py F24 2`: about 115 packets and one end-of-talk. This is the loop waking itself with no key moving.
- [ ] **Step 2: A combination.** `talk_key=134+135`. `hold_keys.py F23+F24 2`: about 100 packets. `hold_keys.py F24 2`: the listener hears nobody.
- [ ] **Step 3: A whisper key never reaches the channel.** A whisper key on F23 aimed at "Everyone, the channel above mine", the app in Radio, listeners in Radio and in Deep Rock. `hold_keys.py F23 2`: Deep Rock hears about 100 whisper packets and an end-of-whisper; Radio hears nothing of either kind.
- [ ] **Step 4: A mute key, however short.** `mute_mic_key=124` (F13). `hold_keys.py F13 0.02`, then a screenshot: the dock reads "Microphone muted". Once more: it does not. Ten presses in a row end where they started.
- [ ] **Step 5: Choosing a key.** Settings, Shortcuts, "Add a key", then `hold_keys.py F22 0.3`, then a screenshot: a chip named "F22". Click a chip and close the settings window without pressing anything, then `hold_keys.py F24 2` with a listener: voice flows, so nothing was left waiting for a key.
- [ ] **Step 6: Held before the app started.** Start `hold_keys.py F24 9` (without `--not-focused`, since the app is not running yet), then start the app with `--connect`, with a listener already in the channel. Expected: voice from the moment the app is connected until the key is let go, then one end-of-talk.
- [ ] **Step 7: Switching while held.** Talk key F24, a listener in the channel, the settings window open on Shortcuts. Start `hold_keys.py F24 4` and, about 1.5 s in, click "Checked every 5 ms" by program. Expected from the listener: voice before the click and after it, at most one end-of-talk in between, a final end-of-talk when the key is let go, and about 200 packets in all. A screenshot afterwards: the dock does not read "Talking". Repeat in the other direction.
- [ ] **Step 8: The file.** After Step 7 the settings file holds `key_reading=poll` or `key_reading=raw` to match the switch. Delete the line and start the app: the switch shows "Checked every 5 ms".
- [ ] **Step 9: The saving, measured.** With the app connected and no key touched for 20 s, run `Get-Counter '\Thread(ps-app*)\Context Switches/sec' -SampleInterval 5 -MaxSamples 2` in PowerShell, once for each position of the switch, and write both lists into the ledger. Expected: with "Checked every 5 ms" one thread shows several hundred a second without pause; with "As they happen" no thread does.
- [ ] **Step 10: Things only you can do.** Each needs real hardware or would disturb the PC it runs on:
  1. A mouse button (4 or 5) as the talk key with "As they happen": talk, then let go.
  2. Hold the talk key, press Win+L, let go of everything, sign in again: the dock must not read "Talking".
  3. With a game in the focus, in a window and in full screen: the talk key works in both positions of the switch.
  4. With a program that runs as administrator in the focus: expected not to work in either position unless Parrotfish is also started as administrator. Say what happened.
  5. If your keyboard has AltGr: a talk key on Left Ctrl is not set off by typing with AltGr in the new way.
- [ ] **Step 11: Documents.** In `PLAN.md`, where it says keys are read by asking Windows 200 times a second: the two ways, which is the default and why, the guard and its 0.1 s bound, the one-listener-per-program rule and the toolkit, the mouse being listened to only when chosen, the limits that both ways share (administrator windows, the lock screen), and the test count. In `README.md`: one line in the feature list. Write the ledger `2026-10-07-raw-input-keys.ledger.md` beside this plan with what was decided on the way and the numbers from Steps 1 to 9. Run `build.bat`.

## Not covered, on purpose

- **Game controllers, pedals and joysticks.** Mumble reads them through the same mechanism with other device kinds. You left them out for now; this plan makes the place where they would go.
- **Keys Windows gives no key code, and mouse buttons beyond five.** They need keys to be stored by another number than the Windows key code, which touches the settings file, the key names and the 5 ms check.
- **Telling two keyboards or two mice apart.** Raw Input says which device a record came from; Parrotfish ignores it.
- **Hiding a talk key from other programs.** That needs a keyboard hook, which this project decided against.
- **Making "As they happen" the default.** Worth deciding after it has been used for a while.
- **Other systems.** Parrotfish's key reading is Windows-only in both ways.
