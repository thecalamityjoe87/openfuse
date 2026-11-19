//! Window / tray behaviour: close-to-tray, meter polling while hidden, tray menu state.

mod common;
use common::*;
use openfuse::backend::Status;
use openfuse::models;
use openfuse::ui::{External, Message};

#[test]
fn hidden_window_stops_meter_polling() {
    use openfuse::backend::mock::MockBackend;
    let mut app = app_with(Box::new(MockBackend::new(&models::MINIFUSE2, "", true, Status::Simulated, Some(1))));
    assert!(app.backend().meters_polling());
    app.fake_tray = true;
    let _ = app.close_window();
    assert!(app.window.is_none() && !app.visible);
    assert!(!app.backend().meters_polling());
    let _ = app.update(Message::External(External::Open));
    assert!(app.window.is_some() && app.visible && app.backend().meters_polling());
}

#[test]
fn close_hides_to_tray_only_when_there_is_one() {
    let mut app = app();
    app.fake_tray = true;
    let id = app.window.unwrap();
    let _ = app.update(Message::CloseRequested(id));
    assert!(app.window.is_none() && !app.quitting);
    assert_eq!(app.notifications.len(), 1, "first close explains the tray once");
    let _ = app.update(Message::External(External::ToggleWindow));
    let id = app.window.expect("tray click shows the window");
    let _ = app.update(Message::CloseRequested(id));
    assert_eq!(app.notifications.len(), 1);

    let mut app = app_for(&models::MINIFUSE2);
    let id = app.window.unwrap();
    let _ = app.update(Message::CloseRequested(id));
    assert!(app.quitting, "no tray: closing quits");
}

#[test]
fn tray_going_away_brings_the_window_back() {
    let mut app = app();
    app.fake_tray = true;
    let _ = app.close_window();
    let _ = app.update(Message::External(External::TrayOffline));
    assert!(app.window.is_some() && !app.has_tray());
}

#[test]
fn tray_menu_mirrors_the_device() {
    let mut app = app();
    let snap = app.tray_snapshot();
    let labels: Vec<&str> = snap.toggles.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, ["INST 1 (Hi-Z)", "INST 2 (Hi-Z)", "48 V phantom power", "Direct Mono", "Loopback"]);
    assert!(snap.toggles.iter().all(|t| t.enabled && !t.checked));
    assert_eq!(snap.status_line, "MiniFuse 2 — simulated");
    assert!(snap.offline && !snap.phantom);
    assert!(snap.tooltip().contains("INST 1 off · INST 2 off · 48 V off"));
    let _ = app.update(Message::External(External::Toggle("in2_inst".into())));
    assert!(app.value_bool("in2_inst"));
    assert!(app.tray_snapshot().toggles[1].checked);
    // a tray toggle is a request, not a local flip: the hardware press path updates it too
    app.mock().simulate_hardware_press("in2_inst");
    let _ = app.update(Message::Tick);
    assert!(!app.tray_snapshot().toggles[1].checked);
}

#[test]
fn quit_from_tray() {
    let mut app = app();
    let _ = app.update(Message::External(External::Quit));
    assert!(app.quitting);
}

#[test]
fn keyboard_shortcuts() {
    use iced::keyboard::key::Named;
    let mut app = app();
    let ch = |c: &str| Message::Key(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Character(c.into()),
        modified_key: iced::keyboard::Key::Character(c.into()),
        physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    });
    let _ = app.update(ch("b"));
    assert_eq!(app.view, models::View::Back);
    let _ = app.update(ch("l"));
    assert!(app.panel_open);
    key(&mut app, Named::Escape);
    assert!(!app.panel_open);
    let _ = app.update(ch("f"));
    assert_eq!(app.view, models::View::Front);
}
