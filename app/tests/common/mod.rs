//! Shared helpers: an App on the mock backend, and a headless iced simulator
//! (tiny-skia) to click through the real view.
#![allow(dead_code)]

use iced::keyboard::{self, key::Named};
use iced::{Point, Size, window};
use iced_test::simulator::{self, Simulator};
use openfuse::autostart::Settings;
use openfuse::backend::Backend;
use openfuse::backend::mock::MockBackend;
use openfuse::geom::Rect;
use openfuse::models::{self, ModelSpec};
use openfuse::ui::{App, Message, ThemeMode, theme};

pub const W: f32 = 1280.0;
pub const H: f32 = 800.0;

pub fn settings_file() -> Settings {
    let dir = tempfile::tempdir().unwrap().keep();
    Settings::at(dir.join("settings.ini"))
}

pub fn app_with(backend: Box<dyn Backend>) -> App {
    unsafe { std::env::set_var("ICED_TEST_BACKEND", "tiny-skia") };
    let mut app = App::new(backend, ThemeMode::Light, settings_file());
    app.window_size = Size::new(W, H);
    app.window = Some(window::Id::unique());
    app.set_visible(true);
    app.start();
    app
}

pub fn app_for(model: &'static ModelSpec) -> App {
    app_with(Box::new(MockBackend::still(model)))
}

pub fn app() -> App {
    app_for(&models::MINIFUSE2)
}

fn iced_settings() -> iced::Settings {
    iced::Settings {
        default_font: theme::INTER,
        fonts: theme::FONTS.iter().map(|f| std::borrow::Cow::Borrowed(*f)).collect(),
        ..Default::default()
    }
}

/// Run `f` against the rendered window and feed the produced messages back.
pub fn interact(app: &mut App, f: impl FnOnce(&mut Simulator<'_, Message>)) {
    let msgs: Vec<Message> = {
        let mut sim = Simulator::with_size(iced_settings(), Size::new(W, H), app.view(window::Id::unique()));
        f(&mut sim);
        sim.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
}

pub fn click_at(app: &mut App, p: Point) {
    interact(app, |sim| {
        sim.point_at(p);
        sim.simulate(simulator::click());
    });
}

pub fn click_text(app: &mut App, label: &str) {
    let label = label.to_string();
    interact(app, move |sim| {
        sim.click(label.as_str()).unwrap_or_else(|e| panic!("cannot click {label:?}: {e:?}"));
    });
}

/// Is a text visible anywhere in the window?
pub fn shows(app: &App, label: &str) -> bool {
    let mut sim = Simulator::with_size(iced_settings(), Size::new(W, H), app.view(window::Id::unique()));
    sim.find(label).is_ok()
}

pub fn center(r: Rect) -> Point {
    Point::new(r.center().x as f32, r.center().y as f32)
}

/// Click a part on the render.
pub fn click_part(app: &mut App, key: &str) {
    let r = app.part_rect(key).expect("part on this view");
    click_at(app, center(r));
}

/// Click the on/off switch of a toggle card (right end of the card).
pub fn click_card_switch(app: &mut App, part_key: &str) {
    let r = app.card_rect(part_key).expect("card");
    click_at(app, Point::new((r.right() - 14.0 - 22.0) as f32, r.center().y as f32));
}

pub fn key(app: &mut App, named: Named) {
    let _ = app.update(Message::Key(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(named),
        modified_key: keyboard::Key::Named(named),
        physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    }));
}

pub fn non_meter_lit(app: &App) -> Vec<String> {
    app.lit_elements().into_iter().filter(|e| !e.starts_with("meter-")).collect()
}
