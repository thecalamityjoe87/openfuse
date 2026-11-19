//! About dialog: credits, opened from the header's "i" button.

mod common;
use common::*;
use iced::Point;
use iced::keyboard::key::Named;
use openfuse::models::View;
use openfuse::ui::{Message, about};

#[test]
fn the_header_button_opens_the_credits() {
    let mut app = app();
    assert!(!app.about_open && !shows(&app, about::CREDIT));
    click_text(&mut app, "i");
    assert!(app.about_open);
    assert!(shows(&app, "Isaac Joseph, Calamity Audio"));
    assert!(shows(&app, about::NAME) && shows(&app, about::TAGLINE) && shows(&app, &about::version()));
    assert!(shows(&app, about::LICENSE));
}

#[test]
fn escape_enter_the_close_button_and_the_backdrop_dismiss_it() {
    let mut app = app();
    for dismiss in 0..4 {
        let _ = app.update(Message::SetAbout(true));
        assert!(shows(&app, "Close"));
        match dismiss {
            0 => key(&mut app, Named::Escape),
            1 => key(&mut app, Named::Enter),
            2 => click_text(&mut app, "Close"),
            _ => click_at(&mut app, Point::new(60.0, 400.0)),
        }
        assert!(!app.about_open, "dismiss way {dismiss}");
    }
}

#[test]
fn it_is_modal_for_the_keyboard() {
    let mut app = app();
    let _ = app.update(Message::SetAbout(true));
    let _ = app.update(Message::Key(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Character("b".into()),
        modified_key: iced::keyboard::Key::Character("b".into()),
        physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    }));
    assert_eq!(app.view, View::Front, "B did not switch to the back view behind the dialog");
    assert!(app.about_open);
}
