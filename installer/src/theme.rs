//! OpenFuse visual design: color tokens, fonts and widget styles.
//!
//! Kept in one place so the installer and the app share the same look.

use iced::font::{Family, Weight};
use iced::widget::{button, checkbox, container, progress_bar, scrollable, text, text_input};
use iced::{border, Background, Border, Color, Font, Shadow, Theme, Vector};

pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
pub const FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
pub const FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");

pub const INTER: Font = Font::with_name("Inter");
pub const MEDIUM: Font = Font { family: Family::Name("Inter"), weight: Weight::Medium, ..Font::DEFAULT };
pub const SEMIBOLD: Font = Font { family: Family::Name("Inter"), weight: Weight::Semibold, ..Font::DEFAULT };
pub const BOLD: Font = Font { family: Family::Name("Inter"), weight: Weight::Bold, ..Font::DEFAULT };

/// Design tokens for one appearance.
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub dark: bool,
    pub bg: Color,
    pub surface: Color,
    pub surface_hover: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub on_accent: Color,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
    pub bad_hover: Color,
    // The sidebar is dark in both appearances.
    pub side_bg: Color,
    pub side_text: Color,
    pub side_muted: Color,
    pub side_line: Color,
}

const fn hex(rgb: u32) -> Color {
    Color::from_rgb(
        ((rgb >> 16) & 0xff) as f32 / 255.0,
        ((rgb >> 8) & 0xff) as f32 / 255.0,
        (rgb & 0xff) as f32 / 255.0,
    )
}

pub const LIGHT: Tokens = Tokens {
    dark: false,
    bg: hex(0xFFFFFF),
    surface: hex(0xF5F6F8),
    surface_hover: hex(0xECEEF2),
    border: hex(0xE4E6EB),
    text: hex(0x14161B),
    muted: hex(0x6A7080),
    accent: hex(0x3A5CF0),
    accent_hover: hex(0x2E4BD1),
    on_accent: hex(0xFFFFFF),
    good: hex(0x1E9E5A),
    warn: hex(0xD48A0C),
    bad: hex(0xD8443A),
    bad_hover: hex(0xBE362D),
    side_bg: hex(0x111319),
    side_text: hex(0xEEF0F5),
    side_muted: hex(0x7D8394),
    side_line: hex(0x2A2E39),
};

pub const DARK: Tokens = Tokens {
    dark: true,
    bg: hex(0x17191F),
    surface: hex(0x1F222A),
    surface_hover: hex(0x272B34),
    border: hex(0x2C303A),
    text: hex(0xECEEF3),
    muted: hex(0x9298A6),
    accent: hex(0x6F8BFF),
    accent_hover: hex(0x8AA0FF),
    on_accent: hex(0x0E1020),
    good: hex(0x3DCB7E),
    warn: hex(0xF2B440),
    bad: hex(0xFF6B61),
    bad_hover: hex(0xFF877E),
    side_bg: hex(0x0E1015),
    side_text: hex(0xEEF0F5),
    side_muted: hex(0x7D8394),
    side_line: hex(0x252935),
};

pub fn tokens(theme: &Theme) -> Tokens {
    if theme.extended_palette().is_dark { DARK } else { LIGHT }
}

pub fn iced_theme(t: Tokens) -> Theme {
    Theme::custom(
        if t.dark { "OpenFuse Dark" } else { "OpenFuse Light" }.to_string(),
        iced::theme::Palette { background: t.bg, text: t.text, primary: t.accent, success: t.good, danger: t.bad },
    )
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

// ---------------------------------------------------------------- containers

pub fn page(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style { background: Some(Background::Color(t.bg)), text_color: Some(t.text), ..Default::default() }
}

pub fn sidebar(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style { background: Some(Background::Color(t.side_bg)), text_color: Some(t.side_text), ..Default::default() }
}

pub fn card(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.surface)),
        border: Border { color: t.border, width: 1.0, radius: 14.0.into() },
        ..Default::default()
    }
}

pub fn hero(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    let top = if t.dark { hex(0x232733) } else { hex(0xEEF1FB) };
    container::Style {
        background: Some(Background::Gradient(iced::Gradient::Linear(
            iced::gradient::Linear::new(std::f32::consts::PI)
                .add_stop(0.0, top)
                .add_stop(1.0, t.surface),
        ))),
        border: Border { color: t.border, width: 1.0, radius: 16.0.into() },
        ..Default::default()
    }
}

pub fn divider(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style { background: Some(Background::Color(t.border)), ..Default::default() }
}

/// A round, tinted badge behind an icon.
pub fn badge(color: Color, strength: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(alpha(color, strength))),
        border: border::rounded(999),
        ..Default::default()
    }
}

pub fn pill(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(alpha(color, 0.16))),
        text_color: Some(color),
        border: border::rounded(999),
        ..Default::default()
    }
}

/// Timeline marker in the sidebar.
pub fn marker(state: Marker) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let t = tokens(theme);
        let accent = DARK.accent; // the sidebar is always dark
        match state {
            Marker::Done => container::Style {
                background: Some(Background::Color(accent)),
                border: border::rounded(999),
                ..Default::default()
            },
            Marker::Current => container::Style {
                background: Some(Background::Color(alpha(accent, 0.18))),
                border: Border { color: accent, width: 2.0, radius: 999.0.into() },
                ..Default::default()
            },
            Marker::Upcoming => container::Style {
                border: Border { color: t.side_line, width: 2.0, radius: 999.0.into() },
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Done,
    Current,
    Upcoming,
}

pub fn line(done: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let t = tokens(theme);
        container::Style {
            background: Some(Background::Color(if done { alpha(DARK.accent, 0.7) } else { t.side_line })),
            ..Default::default()
        }
    }
}

pub fn dot(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(Background::Color(color)), border: border::rounded(999), ..Default::default() }
}

// ---------------------------------------------------------------- text

pub fn muted(theme: &Theme) -> text::Style {
    text::Style { color: Some(tokens(theme).muted) }
}

pub fn accent_text(theme: &Theme) -> text::Style {
    text::Style { color: Some(tokens(theme).accent) }
}

pub fn side_muted(theme: &Theme) -> text::Style {
    text::Style { color: Some(tokens(theme).side_muted) }
}

// ---------------------------------------------------------------- buttons

pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => t.accent_hover,
        button::Status::Disabled => alpha(t.accent, 0.35),
        button::Status::Active => t.accent,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: if status == button::Status::Disabled { alpha(t.on_accent, 0.8) } else { t.on_accent },
        border: border::rounded(10),
        shadow: if status == button::Status::Disabled {
            Shadow::default()
        } else {
            Shadow { color: alpha(t.accent, 0.28), offset: Vector::new(0.0, 4.0), blur_radius: 12.0 }
        },
    }
}

pub fn secondary(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => t.surface_hover,
            _ => t.bg,
        })),
        text_color: t.text,
        border: Border { color: t.border, width: 1.0, radius: 10.0.into() },
        shadow: Shadow::default(),
    }
}

/// Destructive action: as loud as [`primary`], in red.
pub fn danger(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => t.bad_hover,
        button::Status::Disabled => alpha(t.bad, 0.35),
        button::Status::Active => t.bad,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: if status == button::Status::Disabled { alpha(t.on_accent, 0.8) } else { t.on_accent },
        border: border::rounded(10),
        shadow: if status == button::Status::Disabled {
            Shadow::default()
        } else {
            Shadow { color: alpha(t.bad, 0.28), offset: Vector::new(0.0, 4.0), blur_radius: 12.0 }
        },
    }
}

pub fn link(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    button::Style {
        background: None,
        text_color: if status == button::Status::Hovered { t.accent_hover } else { t.accent },
        border: Border::default(),
        shadow: Shadow::default(),
    }
}

// ---------------------------------------------------------------- inputs

#[allow(dead_code)] // shared design system: used by the app
pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let t = tokens(theme);
    let border_color = match status {
        text_input::Status::Focused => t.accent,
        text_input::Status::Hovered => alpha(t.muted, 0.6),
        _ => t.border,
    };
    text_input::Style {
        background: Background::Color(t.bg),
        border: Border { color: border_color, width: if status == text_input::Status::Focused { 2.0 } else { 1.0 }, radius: 10.0.into() },
        icon: t.muted,
        placeholder: alpha(t.muted, 0.8),
        value: t.text,
        selection: alpha(t.accent, 0.3),
    }
}

pub fn check(theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let t = tokens(theme);
    let checked = match status {
        checkbox::Status::Active { is_checked }
        | checkbox::Status::Hovered { is_checked }
        | checkbox::Status::Disabled { is_checked } => is_checked,
    };
    checkbox::Style {
        background: Background::Color(if checked { t.accent } else { t.bg }),
        icon_color: t.on_accent,
        border: Border { color: if checked { t.accent } else { t.border }, width: 1.5, radius: 6.0.into() },
        text_color: Some(t.text),
    }
}

pub fn progress(theme: &Theme) -> progress_bar::Style {
    let t = tokens(theme);
    progress_bar::Style {
        background: Background::Color(t.surface_hover),
        bar: Background::Color(t.accent),
        border: border::rounded(999),
    }
}

pub fn scroll(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut s = scrollable::default(theme, status);
    let t = tokens(theme);
    s.vertical_rail.background = None;
    s.vertical_rail.border = Border::default();
    s.vertical_rail.scroller.color = alpha(t.muted, 0.35);
    s.vertical_rail.scroller.border = border::rounded(999);
    s
}
