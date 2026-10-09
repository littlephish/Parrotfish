#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bookmarks;
mod hotkeys;
mod icons;
mod instance;
mod keywatch;
mod links;
mod mic;
mod platform;
mod scale;
mod session;
mod settings;
mod speakers;
mod update;
mod whisper;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{CloseRequestResponse, ComponentHandle, LogicalSize, Timer, TimerMode};

use app::{with_app, App};
use instance::Wish;
use settings::Settings;

slint::include_modules!();

const UI_TICK: Duration = Duration::from_millis(33);

fn start_wishes(arguments: &[String], scheme: &str) -> Vec<Wish> {
    let mut wishes: Vec<Wish> = Vec::new();
    let mut args = arguments.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--connect" => {
                if let Some(target) = args.next() {
                    wishes.push(Wish::Connect { target: target.clone(), nickname: String::new(), channel: String::new() });
                }
            }
            "--nickname" => {
                if let (Some(name), Some(Wish::Connect { nickname, .. })) = (args.next(), wishes.last_mut()) {
                    *nickname = name.clone();
                }
            }
            "--channel" => {
                if let (Some(name), Some(Wish::Connect { channel, .. })) = (args.next(), wishes.last_mut()) {
                    *channel = name.clone();
                }
            }
            "--update" => wishes.push(Wish::Update),
            link if links::is_link(link, scheme) => wishes.push(Wish::Link(link.to_string())),
            _ => {}
        }
    }
    if wishes.iter().any(|wish| matches!(wish, Wish::Link(_))) {
        wishes.retain(|wish| matches!(wish, Wish::Link(_)));
    }
    wishes
}

fn main() -> Result<(), slint::PlatformError> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let scheme = links::scheme();
    if arguments.iter().any(|arg| arg == "--forget-links") {
        app::forget_links(&scheme);
        return Ok(());
    }
    if let Ok(program) = std::env::current_exe() {
        let working = std::env::current_dir().ok();
        if update::waits_for_the_helper(&program, &Settings::load().updated_from, working.as_deref()) {
            return Ok(());
        }
        if let Some(folder) = program.parent() {
            let _ = std::env::set_current_dir(folder);
        }
    }
    let wishes = start_wishes(&arguments, &scheme);
    let to_hand_over = if wishes.is_empty() { vec![Wish::Show] } else { wishes.clone() };
    let listener = match instance::start(&settings::profile_root(), &to_hand_over) {
        instance::Start::Taken => return Ok(()),
        instance::Start::First(listener) => listener,
    };

    let ui = ParrotfishApp::new()?;
    let settings_window = SettingsWindow::new()?;
    let speakers_window = SpeakersWindow::new()?;
    let settings = Settings::load();
    ui.window().set_size(LogicalSize::new(settings.window_width, settings.window_height));

    let app = Rc::new(RefCell::new(App::new(&ui, &settings_window, &speakers_window, settings)));
    if let (Some(listener), Ok(mut state)) = (listener, app.try_borrow_mut()) {
        state.attach_instance(listener);
    }
    with_app(&app, |state, w| state.start(w, &wishes));

    let a = app.clone();
    ui.on_view_server(move |id| with_app(&a, |s, w| s.view_server(w, id.clamp(0, 0xffff) as u16)));
    let a = app.clone();
    ui.on_connect_bookmark(move |index| with_app(&a, |s, w| s.connect_bookmark(w, index)));
    let a = app.clone();
    ui.on_open_connect(move || with_app(&a, |s, w| s.open_connect(w)));
    let a = app.clone();
    ui.on_connect_new(move || with_app(&a, |s, w| s.connect_new(w)));
    let a = app.clone();
    ui.on_disconnect_viewed(move || with_app(&a, |s, w| s.disconnect_viewed(w)));
    let a = app.clone();
    ui.on_row_activated(move |row| with_app(&a, |s, w| s.row_activated(w, row)));
    let a = app.clone();
    ui.on_row_context(move |row| with_app(&a, |s, w| s.row_context(w, row)));
    let a = app.clone();
    ui.on_channel_join(move || with_app(&a, |s, w| s.channel_join(w)));
    let a = app.clone();
    ui.on_channel_priority_changed(move || with_app(&a, |s, w| s.channel_priority_changed(w)));
    let a = app.clone();
    ui.on_join_with_password(move || with_app(&a, |s, w| s.join_with_password(w)));
    let a = app.clone();
    ui.on_send_chat(move || with_app(&a, |s, w| s.send_chat(w)));
    let a = app.clone();
    ui.on_toggle_mic(move || with_app(&a, |s, w| s.toggle_mic(w)));
    let a = app.clone();
    ui.on_toggle_sound(move || with_app(&a, |s, w| s.toggle_sound(w)));
    let a = app.clone();
    ui.on_open_settings(move |tab| with_app(&a, |s, w| s.open_settings(w, tab)));
    let a = app.clone();
    ui.on_toggle_fold(move |id| with_app(&a, |s, w| s.toggle_fold(w, id)));
    let a = app.clone();
    ui.on_toggle_commander(move || with_app(&a, |s, w| s.toggle_commander(w)));
    let a = app.clone();
    ui.on_toggle_start_here(move || with_app(&a, |s, w| s.toggle_start_here(w)));
    let a = app.clone();
    ui.on_speakers_toggle(move || with_app(&a, |s, w| s.speakers_toggle(w)));
    let a = app.clone();
    ui.on_speakers_lock(move || with_app(&a, |s, w| s.speakers_lock(w)));
    let a = app.clone();
    speakers_window.on_lock_asked(move || with_app(&a, |s, w| s.speakers_lock(w)));
    let a = app.clone();
    speakers_window.on_hide_asked(move || with_app(&a, |s, w| s.speakers_closed(w)));
    let a = app.clone();
    speakers_window.on_dragged(move |dx, dy| with_app(&a, |s, w| s.speakers_dragged(w, dx, dy)));
    let a = app.clone();
    speakers_window.on_sized(move |width, height| with_app(&a, |s, w| s.speakers_sized(w, width, height)));
    let a = app.clone();
    speakers_window.window().on_close_requested(move || {
        with_app(&a, |s, w| s.speakers_closed(w));
        CloseRequestResponse::HideWindow
    });
    let a = app.clone();
    settings_window.on_speakers_changed(move || with_app(&a, |s, w| s.speakers_changed(w)));
    let a = app.clone();
    settings_window.on_links_changed(move || with_app(&a, |s, w| s.links_changed(w)));
    let a = app.clone();
    ui.on_person_voice_changed(move || with_app(&a, |s, w| s.person_voice_changed(w)));
    let a = app.clone();
    ui.on_person_message(move || with_app(&a, |s, w| s.person_message(w)));
    let a = app.clone();
    ui.on_person_poke_sent(move || with_app(&a, |s, w| s.person_poke(w)));
    let a = app.clone();
    ui.on_person_away_toggled(move || with_app(&a, |s, w| s.person_away(w)));
    let a = app.clone();
    ui.on_person_ask_toggled(move || with_app(&a, |s, w| s.person_ask(w)));
    let a = app.clone();
    ui.on_ask_privilege_key(move || with_app(&a, |s, w| s.ask_privilege_key(w)));
    let a = app.clone();
    ui.on_update_acted(move || with_app(&a, |s, w| s.update_acted(w)));
    let a = app.clone();
    ui.on_update_hidden(move || with_app(&a, |s, w| s.update_hidden(w)));
    let a = app.clone();
    settings_window.on_update_acted(move || with_app(&a, |s, w| s.update_acted(w)));
    let a = app.clone();
    settings_window.on_update_check(move || with_app(&a, |s, w| s.look_for_update(w)));
    let a = app.clone();
    settings_window.on_check_updates_changed(move || with_app(&a, |s, w| s.updates_setting_changed(w)));

    let a = app.clone();
    settings_window.on_audio_changed(move || with_app(&a, |s, w| s.apply_audio(w)));
    let a = app.clone();
    settings_window.on_input_device_selected(move || with_app(&a, |s, w| s.select_input(w)));
    let a = app.clone();
    settings_window.on_output_device_selected(move || with_app(&a, |s, w| s.select_output(w)));
    let a = app.clone();
    settings_window.on_new_identity(move || with_app(&a, |s, w| s.new_identity(w)));
    let a = app.clone();
    settings_window.on_import_identity(move || with_app(&a, |s, w| s.import_identity(w)));
    let a = app.clone();
    settings_window.on_bookmark_picked(move |index| with_app(&a, |s, w| s.bookmark_picked(w, index)));
    let a = app.clone();
    settings_window.on_bookmark_new(move || with_app(&a, |s, w| s.bookmark_new(w)));
    let a = app.clone();
    settings_window.on_bookmark_saved(move || with_app(&a, |s, w| s.bookmark_saved(w)));
    let a = app.clone();
    settings_window.on_bookmark_removed(move || with_app(&a, |s, w| s.bookmark_removed(w)));
    let a = app.clone();
    settings_window.on_bookmark_channel_selected(move || with_app(&a, |s, w| s.bookmark_channel_selected(w)));
    let a = app.clone();
    settings_window.on_talk_key_change(move |index| with_app(&a, |s, w| s.talk_key_change(w, index)));
    let a = app.clone();
    settings_window.on_talk_key_add(move || with_app(&a, |s, w| s.talk_key_add(w)));
    let a = app.clone();
    settings_window.on_talk_key_remove(move |index| with_app(&a, |s, w| s.talk_key_remove(w, index)));
    let a = app.clone();
    settings_window.on_shortcut_changed(move || with_app(&a, |s, w| s.shortcut_changed(w)));
    let a = app.clone();
    settings_window.on_whisper_key_add(move || with_app(&a, |s, w| s.whisper_key_add(w)));
    let a = app.clone();
    settings_window.on_whisper_key_edit(move |index| with_app(&a, |s, w| s.whisper_key_edit(w, index)));
    let a = app.clone();
    settings_window.on_whisper_key_remove(move |index| with_app(&a, |s, w| s.whisper_key_remove(w, index)));
    let a = app.clone();
    settings_window.on_whisper_key_change(move |index| with_app(&a, |s, w| s.whisper_key_change(w, index)));
    let a = app.clone();
    settings_window.on_reply_key_change(move || with_app(&a, |s, w| s.reply_key_change(w)));
    let a = app.clone();
    settings_window.on_reply_key_clear(move || with_app(&a, |s, w| s.reply_key_clear(w)));
    let a = app.clone();
    settings_window.on_action_key_change(move |which| with_app(&a, |s, w| s.action_key_change(w, which)));
    let a = app.clone();
    settings_window.on_action_key_clear(move |which| with_app(&a, |s, w| s.action_key_clear(w, which)));
    let a = app.clone();
    settings_window.on_editor_key_change(move || with_app(&a, |s, w| s.editor_key_change(w)));
    let a = app.clone();
    settings_window.on_editor_key_clear(move || with_app(&a, |s, w| s.editor_key_clear(w)));
    let a = app.clone();
    settings_window.on_editor_toggle(move |index| with_app(&a, |s, w| s.editor_toggle(w, index)));
    let a = app.clone();
    settings_window.on_editor_changed(move || with_app(&a, |s, w| s.editor_changed(w)));
    let a = app.clone();
    settings_window.on_editor_save(move || with_app(&a, |s, w| s.editor_save(w)));
    let a = app.clone();
    settings_window.on_editor_cancel(move || with_app(&a, |s, w| s.editor_cancel(w)));
    let a = app.clone();
    settings_window.on_view_changed(move || with_app(&a, |s, w| s.view_changed(w)));
    let a = app.clone();
    settings_window.on_done(move || with_app(&a, |s, w| s.close_settings(w)));

    let a = app.clone();
    settings_window.window().on_close_requested(move || {
        with_app(&a, |s, w| s.settings_hidden(w));
        CloseRequestResponse::HideWindow
    });
    let a = app.clone();
    ui.window().on_close_requested(move || {
        with_app(&a, |s, w| s.leave(w));
        CloseRequestResponse::HideWindow
    });

    let timer = Timer::default();
    let a = app.clone();
    timer.start(TimerMode::Repeated, UI_TICK, move || with_app(&a, |s, w| s.tick(w)));

    ui.run()?;
    timer.stop();
    if let Ok(mut state) = app.try_borrow_mut() {
        state.shutdown();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(arguments: &[&str]) -> Vec<Wish> {
        let arguments: Vec<String> = arguments.iter().map(|argument| argument.to_string()).collect();
        start_wishes(&arguments, "ts3server")
    }

    fn connect(target: &str, nickname: &str, channel: &str) -> Wish {
        Wish::Connect { target: target.to_string(), nickname: nickname.to_string(), channel: channel.to_string() }
    }

    #[test]
    fn the_command_line_is_read_into_what_was_asked_for() {
        assert_eq!(asked(&[]), Vec::new());
        assert_eq!(
            asked(&["--connect", "Reef Runners", "--nickname", "Pike", "--channel", "Deep Rock", "--connect", "other.example.org:9988"]),
            vec![connect("Reef Runners", "Pike", "Deep Rock"), connect("other.example.org:9988", "", "")]
        );
        assert_eq!(asked(&["--update"]), vec![Wish::Update]);
        assert_eq!(asked(&["--connect", "Reef", "--update"]), vec![connect("Reef", "", ""), Wish::Update]);
        assert_eq!(asked(&["--nickname", "Pike", "--loud", "--connect"]), Vec::new(), "orders with nothing to apply to");
        assert_eq!(asked(&["ts3server://example.org?port=9988"]), vec![Wish::Link("ts3server://example.org?port=9988".to_string())]);
    }

    #[test]
    fn a_link_cannot_bring_orders_of_its_own() {
        let link = Wish::Link("ts3server://example.org".to_string());
        for smuggled in [
            &["ts3server://example.org", "--connect", "evil.example", "--nickname", "Admin"][..],
            &["ts3server://example.org", "--update"][..],
            &["--update", "ts3server://example.org"][..],
            &["--connect", "evil.example", "ts3server://example.org", "--channel", "Trap"][..],
        ] {
            assert_eq!(asked(smuggled), vec![link.clone()], "{smuggled:?}");
        }
    }
}
