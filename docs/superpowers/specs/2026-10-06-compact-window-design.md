# Parrotfish compact window: design

Date: 2026-10-06. Status: waiting for your review. Nothing in here is built yet.

Mock-ups: `.superpowers/brainstorm/1131-1791292647/content/compact-detail.html` (the page open in
your browser) and `window-layouts-v3.html` (the three layouts you chose from).

## What you asked for

- A window modelled on good voice clients (TeamSpeak 3, Mumble, Discord). You picked the
  **compact tree** layout and the **Twilight reef** palette.
- Every setting in one settings panel with tabs.
- Spacer channels drawn as real dividers instead of raw `[cspacer]` text.
- Server bookmarks for quick connections.
- More than one server connected at once, as in TeamSpeak 3.
- System fonts only. No typeface is bundled with the app.

## Decisions I made that you have not confirmed

Say so if any of these should change. I will build to them otherwise.

1. **"[Cspace]" means spacer channels.** `[cspacer]Games`, `[*spacer1]---` and friends.
2. **Audio with several servers.** You hear every connected server. Your microphone goes only to
   the server you are viewing. Mute microphone and mute sound apply to all of them.
3. **Settings open in their own window.** The compact window is about 400 px wide and six tabs
   need about 650. The mock-up on the first page showed the panel inside a wide window, so this
   differs from what you saw there.
4. **Bookmarks do not store server passwords in this round.** Parrotfish asks when a server
   wants one. Saving them safely needs Windows credential encryption, which I would add next
   rather than store passwords as plain text now.

## The window

About 400 x 740 by default, resizable down to 340 x 520. It remembers its size.

```
+--------------------------------------+
| [RR] Reef Runners          [NS]   v  |  server header: click for the servers menu
|      12 people online                |  small tiles: your other connected servers
+--------------------------------------+
|  --------- Reef Runners ---------    |  spacer, centred
|  ))  Lobby                        3  |  channel you are in
|        o LittlePhish                 |  you
|        * Marlin                      |  amber dot: talking
|        o Coralline             (mic) |  microphone muted
|  ----------------------------------  |  spacer, line
|  ------------- Games -------------   |
|  ))  Deep Rock                    2  |
|  [lock] Squad Alpha                  |
|  ...                                 |
+--------------------------------------+
| 20:18 Coralline found it, joining... |  chat drawer, closed: last message
| [ Message Lobby                    ] |
+--------------------------------------+
| [LP] LittlePhish     (mic)(ear)(cog) |  dock
|      Sends when I speak              |
| [=======|-------------------] 23 ms  |  level meter, speaking marker, ping
+--------------------------------------+
```

**Server header.** Shows the server you are viewing: a tile with its initials, its name, and how
many people are online. Up to three small tiles show your other connected servers; an amber dot
on one means someone is talking there. Clicking a small tile switches to that server. Clicking
the name opens the servers menu, which lists every connection, including any beyond those three.

**Servers menu.** Connected servers first (click to view), then bookmarks (click to connect),
then three actions: Connect to a server, Edit bookmarks, Disconnect from the viewed server.

**Channel tree.** One row per spacer, channel or person, 26 px high.
- Channels show a speaker icon, a lock if they need a password, or a note for music channels,
  and the number of people inside. The channel you are in is highlighted.
- People show a dot (grey idle, amber with a ring while talking), their name (yours in bold),
  and at the right a crossed microphone or headphones if muted, or "away".
- Double-click a channel to join it. A locked channel asks for its password in a small prompt
  over the window; the permanent "Channel password" box goes away.

**Chat drawer.** Closed, it shows the newest message and the message box. Click the message and
it opens to about half the window with history and a Lobby / Server switch. Joins, leaves and
errors appear as dim lines. Each server keeps its own history (the last 400 lines).

**Dock.** Your initials, nickname and one status line, then three buttons: mute microphone,
mute sound, settings. Below, the microphone level with the speaking marker, and the ping.
The status line says, in this order of priority: Not connected, Connecting, Sound muted,
Microphone muted, Talking, then the send mode in the same words the settings use: "Sends when I
speak", "Sends while I hold Left Ctrl", or "Always sends".

**No server yet.** The tree area invites the first step: a Connect to a server button and your
bookmarks listed underneath, each one click away.

**When something fails.** The message says what happened and what to do, next to where it
happened: under the address field in the connect dialog ("No answer from reef.example.net. Check
the address and port."), or as a dim line in chat for errors from a server you are already on.

## Bookmarks and connections

A bookmark holds a name, the server address, the nickname to use and which identity to use.
Bookmarks live in `%APPDATA%\Parrotfish\bookmarks.ini`. The tile shows the initials of the name.

Connect to a server opens a small dialog: address, nickname, optional server password,
identity, and a "Save as a bookmark" box. Edit bookmarks opens the Bookmarks tab of settings.

Each connection is independent: its own channel tree, chat history and connection state.
Disconnecting one leaves the others alone. Closing the window disconnects all of them.
If the server you are viewing disconnects, the view moves to another connected server, or to
the empty state when none is left. If you switch servers while talking, the server you leave is
told you stopped, so you do not appear stuck talking there.

## Spacer channels

A top-level channel named `[<align>spacer<anything>]<text>` is drawn as a spacer, not a channel:

| Name | Drawn as |
|---|---|
| `[cspacer]Games` | "Games" centred between two thin rules |
| `[lspacer]Chill`, `[spacer]Chill` | "Chill" at the left |
| `[rspacer]est. 2019` | text at the right |
| `[*spacer]-=` | the text repeated across the width |
| text `---`, `-.-`, `-..` | a dashed rule |
| text `...` | a dotted rule |
| text `___` | a solid rule |
| no text | an empty gap |

Anything after `spacer` up to the `]` (TeamSpeak uses numbers there to keep names unique) is
ignored. Channels below the top level are never spacers. Spacers cannot be joined.

## Settings window

Its own window, about 700 x 470, opened from the cog. Changes apply as you make them; the one
button is Done.

- **Microphone.** Device. Send my voice: When I speak / While I hold a key / Always. Speaking
  level with a live meter and a draggable marker. Microphone boost. Play my microphone back to me.
- **Sound.** Device and volume.
- **Identities.** Your identities with nickname and security level. Create one, or import a
  TeamSpeak identity file.
- **Bookmarks.** The list, with name, address, nickname and identity editable, and Remove.
- **Shortcuts.** The key to hold for talking.
- **About.** Version, and the credits from the README.

## Look

Palette, Twilight reef:

| Name | Value | Used for |
|---|---|---|
| Trench | `#10222E` | title bar, dock, message box: the deepest surfaces |
| Reef | `#18313F` | the window |
| Shoal | `#1F3D4E` | chat drawer, menus, raised surfaces |
| Current | `#26495E` | hover and the selected row |
| Foam | `#E9F2F1` | text |
| Drift | `#8DA9B3` | secondary text and resting icons |
| Lure | `#FFC043` | someone is talking; the viewed server; the main button |
| Coral | `#F0705F` | muted, errors, disconnect |

Lure is the one bright thing in the window and it always means the same: sound is happening
here, or this is where you are. Nothing decorative uses it.

Type: the system font (Segoe UI on Windows) at 13 px for rows, 13.5 px for chat, 12 px for
secondary text, weights 400 and 600. The mock-ups were drawn in Rubik, so letterforms in the
app will look a little plainer than in the pictures; sizes, spacing and colours are unchanged.
No capitalised labels. Icons are small line drawings (microphone, headphones, cog, lock,
speaker, note, plus, bookmark, close, chevron, signal), tinted with the text colours. When
someone starts talking their dot gets one short ring; there is no other animation.

Checks before I call it done: text contrast of at least 4.5:1 on every surface, a visible
keyboard focus ring, a hover state on everything clickable, and no control smaller than 28 px.

## How the code changes

- `ps-client`: new `spacer` module that turns a channel name into a spacer description, with tests.
- `ps-voice`: the mixer identifies a talker by connection and client, not client alone, so two
  servers can reuse the same client number. Adds "forget everyone from this connection".
- `ps-app` is split so each file has one job: `session.rs` (one connection: its client, events,
  tree rows, chat history), `bookmarks.rs` (the list and its file), `app.rs` (all sessions,
  which one is viewed, where the microphone goes), `main.rs` (start-up).
- UI files: `theme.slint` (the palette and sizes), `widgets.slint` (tile, icon button, meter,
  row types), `main.slint` (the compact window), `settings.slint` (the settings window),
  plus `icons/`.
- The current single-window layout is replaced, not kept alongside.

## How I will test it

- Unit tests: spacer names (every row of the table above plus near-misses), bookmark file round
  trip, two connections with the same client number in the mixer, tree rows built from a view
  that contains spacers.
- Against the local TeamSpeak server: create spacer channels and check them in a screenshot;
  connect two sessions at once; confirm with a headless listener that the microphone reaches
  only the viewed server and switches when the view switches; confirm voice from both servers
  is heard; bookmark, quit, restart, reconnect from the bookmark.
- Screenshots of every state in the mock-up page, compared against it.

## Not in this round

Saved server passwords, a default channel per bookmark, connecting automatically at start-up,
a details pane for the selected channel or person, private-chat tabs, per-person volume sliders,
keep-window-on-top, a light theme, whispers, channel editing.
