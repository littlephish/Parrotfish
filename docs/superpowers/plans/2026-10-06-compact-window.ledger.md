# Compact window: what was done, decided and checked

Date: 2026-10-06. Plan: `2026-10-06-compact-window.md`. Spec: `../specs/2026-10-06-compact-window-design.md`.

All six tasks are finished. This file records the decisions I made without asking (each one is a
"Ruling"), what was checked and how, and what was not checked.

## Result

- 120 unit tests pass (`cargo test --workspace`); `cargo build --workspace --all-targets` has no warnings.
- The new window was driven with real clicks and keystrokes against a TeamSpeak 3.13.8 server and
  checked in screenshots. The microphone routing between two connections was checked packet by
  packet with headless listeners.

## Rulings

Changes to how the plan was carried out:

1. Ruling: tests were written together with the code, not watched failing first. It saved time;
   every test listed in the plan exists and passes.
2. Ruling: the settings window uses the same custom widgets as the main window (field, dropdown,
   slider, tick box) and not the stock Slint ones the plan named. The stock ones cannot take the
   palette, and two looks in one app would have been worse.
3. Ruling: UI callbacks carry no arguments; Rust reads the values from the bound properties. The
   plan listed argument lists. Same behaviour, less glue.
4. Ruling: the screenshots were compared with the mock-up page by its content and the spec, not
   pixel by pixel. One difference found and fixed: the servers menu now says "viewing" and
   "someone is talking" as the mock-up did.

Behaviour the spec did not settle:

5. Ruling: a spacer channel that has people in it still lists them under the divider.
6. Ruling: a bookmark is saved when the connection succeeds, not when you click Connect, and is
   named after the server. An existing bookmark keeps its name.
7. Ruling: one bookmark per server address, as planned. A second bookmark for the same server
   needs a different spelling of the address, for example with the port.
8. Ruling: if a connection fails while the connect dialog is already open for something else, the
   reason appears in the banner under the header and the dialog is left alone.
9. Ruling: joins and leaves of ServerQuery clients are not written to chat, and BBCode tags are
   removed from chat and welcome text.
10. Ruling: the hint that the microphone is sending silence is kept from the old window. It shows
    once per run in the banner and continuously under the device in settings.
11. Ruling: `--connect` also accepts a bookmark name, and `--channel` was added. I needed both to
    script the two-connection test. A default channel per bookmark is still not in this round.
12. Ruling: the nickname offered for new connections changes only when you connect through the
    dialog, not when a bookmark with its own nickname is used.

Things removed or lost:

13. Ruling: the on-screen "hold to talk" button of the old window is gone. It was not in the spec.
    Hold-to-talk works with the key only.
14. Ruling: chat history is drawn as styled rows. **Text in it can no longer be selected or
    copied**; the old log was a text box where it could. This is a regression to decide on.

Look:

15. Ruling: amber is kept to its three meanings. Tick boxes, plain slider fills and the "Save
    bookmark" button were amber at first and were changed to neutral colours.
16. Ruling: two tints were added to the palette: a lighter grey-blue (`#A4BBC2`) for secondary
    text on the highlighted row, and a lighter coral (`#EDA499`) for error text on the chat drawer
    and menus. The eight agreed colours gave 3.85:1 and 3.92:1 there; the spec asks for 4.5:1.
17. Ruling: tree rows stay 26 px high, as the layout section says, although the quality bar says
    no control under 28 px. Every button, field and switch is 28 px or more.

Found during the live tests and fixed:

18. Ruling: on mute, the end-of-talk packet is now sent before the server is told we are muted.
    The server drops voice from muted clients, so in the other order listeners never got it.
19. Ruling: Escape now closes the menu and dialogs even when no text field has focus.

Changed because of the identity incident on the same day:

20. Ruling: importing an identity remembers where the file is. It is not copied into the app's
    folder, so a private key exists in one place only.
21. Ruling: PhishSpeak no longer looks for a particular identity file by name at start-up. With
    no identity it creates one, with the nickname "PhishSpeakUser".
22. Ruling: the live tests used generated identities only.

About the test itself:

23. Ruling: "several servers at once" was tested as two connections to one server with two
    identities, in two channels. A second TeamSpeak server on this machine would have meant
    getting around the server's one-instance check, which I did not do.
24. Ruling: a new dev tool, `ps-voice/examples/channeltest.rs`, sits in a channel and prints
    every voice and end-of-talk packet it hears, and can send a tone.

## Evidence for the five review points

1. Switching the viewed server while talking. Listener in each channel, app sending continuously.
   Click at 15:08:25.8: the channel left behind received "end-of-talk packet … after 452 packets"
   at 15:08:26.063, the other channel its first packet at 15:08:26.075. Same on the way back
   (15:08:34.182 and 15:08:34.196). The channel that was not viewed received 0 packets for a minute.
2. The viewed server disconnects. Disconnect clicked at 15:10:15.5; the view moved to the other
   connection and its channel received the first packet at 15:10:15.736.
3. Two connections, same client number. Unit test `two_connections_can_share_a_client_number`.
   Live, a talker in each channel was shown at the same moment: one in the tree, one as the amber
   dot on the other connection's tile.
4. Names that only look like spacers. Unit tests, and live: a channel named `[cspacer` was shown
   as a channel and joined by double-click.
5. A damaged bookmarks file. Unit test `damaged_file_keeps_complete_entries`.

## Not checked

- Sound by ear. The tests ran with the output volume at 0.
- The hold-to-talk key being held.
- Two different servers at once (see ruling 23).
- A conversation with the official TeamSpeak client.
- The minimum window size by dragging the border. It is declared as 340 × 520.
- The settings window was checked by screenshot and clicks; its sliders were not dragged.
