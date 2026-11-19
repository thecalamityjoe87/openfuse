//! 48 V must be confirmed before it is switched on.

mod common;
use common::*;
use iced::keyboard::key::Named;
use openfuse::backend::Value;
use openfuse::ui::{ConfirmButton, External, Message, phantom};

#[test]
fn enable_asks_and_respects_cancel() {
    let mut app = app();
    let _ = app.update(Message::Toggle("phantom".into(), true));
    assert_eq!(app.confirm, Some(ConfirmButton::Cancel), "dialog open, Cancel focused");
    assert_eq!(app.backend().get("phantom"), Some(Value::Bool(false)), "nothing sent while asking");
    let _ = app.update(Message::Confirm(false));
    assert!(app.confirm.is_none() && !app.value_bool("phantom"));
}

#[test]
fn enable_after_confirmation() {
    let mut app = app();
    let _ = app.update(Message::Toggle("phantom".into(), true));
    let _ = app.update(Message::Confirm(true));
    assert_eq!(app.backend().get("phantom"), Some(Value::Bool(true)));
    assert!(app.lit_elements().contains(&"phantom-48v".to_string()));
}

#[test]
fn disable_does_not_ask() {
    let mut app = app();
    let _ = app.update(Message::Toggle("phantom".into(), true));
    let _ = app.update(Message::Confirm(true));
    click_card_switch(&mut app, "phantom-48v");
    assert!(app.confirm.is_none());
    assert_eq!(app.backend().get("phantom"), Some(Value::Bool(false)));
}

#[test]
fn card_switch_and_render_click_also_ask() {
    let mut app = app();
    click_card_switch(&mut app, "phantom-48v");
    assert!(app.confirm.is_some() && !app.value_bool("phantom"));
    let _ = app.update(Message::Confirm(false));
    click_part(&mut app, "phantom-48v");
    assert!(app.confirm.is_some() && !app.value_bool("phantom"));
}

#[test]
fn tray_toggle_asks_too() {
    let mut app = app();
    let _ = app.update(Message::External(External::Toggle("phantom".into())));
    assert!(app.confirm.is_some() && !app.value_bool("phantom"));
}

#[test]
fn hardware_button_needs_no_confirmation() {
    let mut app = app();
    app.mock().simulate_hardware_press("phantom");
    let _ = app.update(Message::Tick);
    assert!(app.confirm.is_none());
    assert!(app.value_bool("phantom"));
    assert!(app.tray_snapshot().phantom, "tray icon shows the 48 V variant");
}

#[test]
fn real_dialog_cancel_then_accept() {
    let mut app = app();
    click_card_switch(&mut app, "phantom-48v");
    assert!(shows(&app, phantom::TITLE));
    // the dialog is modal: the cards under it do not react
    click_card_switch(&mut app, "in1-inst");
    assert!(!app.value_bool("in1_inst"));
    click_text(&mut app, phantom::CANCEL);
    assert!(app.confirm.is_none() && !app.value_bool("phantom"));
    assert!(!shows(&app, phantom::TITLE));

    click_card_switch(&mut app, "phantom-48v");
    click_text(&mut app, phantom::ENABLE);
    assert!(app.value_bool("phantom"));
}

#[test]
fn enter_activates_the_default_cancel() {
    let mut app = app();
    let _ = app.update(Message::Toggle("phantom".into(), true));
    key(&mut app, Named::Enter);
    assert!(app.confirm.is_none() && !app.value_bool("phantom"));
    // Tab moves the focus to "Turn on 48 V"; Enter then confirms
    let _ = app.update(Message::Toggle("phantom".into(), true));
    key(&mut app, Named::Tab);
    assert_eq!(app.confirm, Some(ConfirmButton::Enable));
    key(&mut app, Named::Enter);
    assert!(app.value_bool("phantom"));
}

#[test]
fn escape_cancels() {
    let mut app = app();
    let _ = app.update(Message::Toggle("phantom".into(), true));
    key(&mut app, Named::Tab);
    key(&mut app, Named::Escape);
    assert!(app.confirm.is_none() && !app.value_bool("phantom"));
}

// ---------------------------------------------------------------- "Don't ask again"

use openfuse::autostart::Settings;
use openfuse::backend::mock::MockBackend;
use openfuse::models;
use openfuse::ui::{App, ThemeMode};

fn app_with_settings(path: &std::path::Path) -> App {
    let mut a = App::new(Box::new(MockBackend::still(&models::MINIFUSE2)), ThemeMode::Light, Settings::at(path.to_path_buf()));
    a.window_size = iced::Size::new(W, H);
    a.window = Some(iced::window::Id::unique());
    a.set_visible(true);
    a.start();
    a
}

#[test]
fn dont_ask_again_is_unticked_and_saved_only_on_confirm() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("openfuse").join("settings.ini");
    let mut app = app_with_settings(&file);
    assert!(app.ask_phantom(), "asks by default");
    let _ = app.update(Message::Toggle("phantom".into(), true));
    assert!(!app.confirm_dont_ask, "unticked by default");
    assert!(shows(&app, phantom::DONT_ASK));
    let _ = app.update(Message::ConfirmDontAsk(true));
    let _ = app.update(Message::Confirm(false)); // Cancel never saves it
    assert!(app.ask_phantom() && !file.exists());
    // reopened: unticked again
    let _ = app.update(Message::Toggle("phantom".into(), true));
    assert!(!app.confirm_dont_ask);
    click_text(&mut app, phantom::DONT_ASK);
    assert!(app.confirm_dont_ask);
    click_text(&mut app, phantom::ENABLE);
    assert!(app.value_bool("phantom") && !app.ask_phantom());
    assert!(file.is_file(), "settings file created on first write");
    // persists for the next start
    assert!(!app_with_settings(&file).ask_phantom());
}

#[test]
fn without_asking_48v_switches_on_directly() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    Settings::at(file.clone()).set_bool(openfuse::ui::KEY_ASK_PHANTOM, false).unwrap();
    let mut app = app_with_settings(&file);
    click_card_switch(&mut app, "phantom-48v");
    assert!(app.confirm.is_none() && app.value_bool("phantom"), "card switch");
    let _ = app.update(Message::Toggle("phantom".into(), false));
    click_part(&mut app, "phantom-48v");
    assert!(app.confirm.is_none() && app.value_bool("phantom"), "render click");
    let _ = app.update(Message::Toggle("phantom".into(), false));
    let _ = app.update(Message::External(External::Toggle("phantom".into())));
    assert!(app.confirm.is_none() && app.value_bool("phantom"), "tray");
}

#[test]
fn asking_can_be_turned_back_on_from_card_and_tray() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    Settings::at(file.clone()).set_bool(openfuse::ui::KEY_ASK_PHANTOM, false).unwrap();
    let mut app = app_with_settings(&file);
    assert!(!app.tray_snapshot().ask_phantom);
    click_text(&mut app, "Ask before turning on"); // checkbox on the 48 V card
    assert!(app.ask_phantom() && app.tray_snapshot().ask_phantom);
    let _ = app.update(Message::Toggle("phantom".into(), true));
    assert!(app.confirm.is_some(), "asks again");
    let _ = app.update(Message::Confirm(false));
    let _ = app.update(Message::External(External::SetAskPhantom(false)));
    assert!(!app.ask_phantom());
    let _ = app.update(Message::External(External::SetAskPhantom(true)));
    assert!(app.ask_phantom());
}

#[test]
fn corrupt_settings_file_falls_back_to_asking() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.ini");
    std::fs::write(&file, b"\xff\xfe\x00garbage\nask_before_phantom\n=\n[x").unwrap();
    let mut app = app_with_settings(&file);
    assert!(app.ask_phantom());
    let _ = app.update(Message::Toggle("phantom".into(), true));
    assert!(app.confirm.is_some());
    std::fs::write(&file, "ask_before_phantom=maybe\n").unwrap();
    assert!(app.ask_phantom(), "unreadable value: default (ask)");
    // a write repairs the file
    let _ = app.update(Message::SetAskPhantom(false));
    assert!(!app.ask_phantom());
    assert!(std::fs::read_to_string(&file).unwrap().contains("ask_before_phantom=false"));
}
