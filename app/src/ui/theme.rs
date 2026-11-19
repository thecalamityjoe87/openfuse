//! OpenFuse visual design: color tokens, fonts and widget styles.
//!
//! Shared with the installer (installer/src/theme.rs) so both binaries look
//! like one product: same tokens, same Inter type, same button / input /
//! checkbox / progress / scroll styles. Differences from the installer copy
//! are limited to iced 0.14 API updates and the "app additions" section at
//! the end (pill switch, slider, segmented control, window chrome).

use iced::font::{Family, Weight};
use iced::widget::{button, checkbox, container, pick_list, progress_bar, scrollable, slider, text, text_input, toggler};
use iced::overlay::menu;
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector, border};

pub const FONT_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
pub const FONT_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
pub const FONT_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
pub const FONT_BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");
pub const FONTS: [&[u8]; 4] = [FONT_REGULAR, FONT_MEDIUM, FONT_SEMIBOLD, FONT_BOLD];

pub const INTER: Font = Font::with_name("Inter");
pub const MEDIUM: Font = Font { family: Family::Name("Inter"), weight: Weight::Medium, ..Font::DEFAULT };
pub const SEMIBOLD: Font = Font { family: Family::Name("Inter"), weight: Weight::Semibold, ..Font::DEFAULT };
pub const BOLD: Font = Font { family: Family::Name("Inter"), weight: Weight::Bold, ..Font::DEFAULT };

/// Design tokens for one appearance.
#[derive(Debug, Clone, Copy, PartialEq)]
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
        iced::theme::Palette {
            background: t.bg,
            text: t.text,
            primary: t.accent,
            success: t.good,
            warning: t.warn,
            danger: t.bad,
        },
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

pub fn card(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(if t.dark { t.surface } else { Color::WHITE })),
        border: Border { color: t.border, width: 1.0, radius: 16.0.into() },
        shadow: Shadow {
            color: alpha(Color::BLACK, if t.dark { 0.35 } else { 0.07 }),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 14.0,
        },
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
        snap: true,
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
        snap: true,
    }
}

pub fn danger(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => alpha(t.bad, 0.10),
            _ => t.bg,
        })),
        text_color: t.bad,
        border: Border { color: alpha(t.bad, 0.45), width: 1.0, radius: 10.0.into() },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn link(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    button::Style {
        background: None,
        text_color: if status == button::Status::Hovered { t.accent_hover } else { t.accent },
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}

// ---------------------------------------------------------------- inputs

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let t = tokens(theme);
    let focused = matches!(status, text_input::Status::Focused { .. });
    let border_color = match status {
        text_input::Status::Focused { .. } => t.accent,
        text_input::Status::Hovered => alpha(t.muted, 0.6),
        _ => t.border,
    };
    text_input::Style {
        background: Background::Color(t.bg),
        border: Border { color: border_color, width: if focused { 2.0 } else { 1.0 }, radius: 10.0.into() },
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
    s.vertical_rail.scroller.background = Background::Color(alpha(t.muted, 0.35));
    s.vertical_rail.scroller.border = border::rounded(999);
    s
}

// ================================================================ app additions

impl Tokens {
    /// Analog / passive callouts and leader lines.
    pub fn inert(&self) -> Color {
        alpha(self.muted, if self.dark { 0.55 } else { 0.6 })
    }
}

/// The on/off switch (shows device state; clicking requests a change).
pub fn switch(danger: bool) -> impl Fn(&Theme, toggler::Status) -> toggler::Style {
    move |theme, status| {
        let t = tokens(theme);
        let acc = if danger { t.bad } else { t.accent };
        let (on, enabled, hover) = match status {
            toggler::Status::Active { is_toggled } => (is_toggled, true, false),
            toggler::Status::Hovered { is_toggled } => (is_toggled, true, true),
            toggler::Status::Disabled { is_toggled } => (is_toggled, false, false),
        };
        let lit = on && enabled;
        toggler::Style {
            background: if lit {
                acc.into()
            } else if hover {
                alpha(t.muted, 0.45).into()
            } else {
                alpha(t.muted, if enabled { 0.30 } else { 0.15 }).into()
            },
            background_border_width: 0.0,
            background_border_color: Color::TRANSPARENT,
            foreground: if enabled { Color::WHITE } else { alpha(Color::WHITE, 0.7) }.into(),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: None,
            border_radius: None,
            padding_ratio: 0.12,
        }
    }
}

/// Slider with an accent fill and a white, ringed handle.
pub fn fader(enabled: bool) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |theme, status| {
        let t = tokens(theme);
        let hot = matches!(status, slider::Status::Hovered | slider::Status::Dragged);
        let fill = if enabled { t.accent } else { t.inert() };
        slider::Style {
            rail: slider::Rail {
                backgrounds: (fill.into(), t.surface_hover.into()),
                width: 4.0,
                border: border::rounded(999),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: if hot { 8.0 } else { 7.0 } },
                background: Color::WHITE.into(),
                border_width: 2.0,
                border_color: if enabled { fill } else { t.border },
            },
        }
    }
}

/// A pill-shaped toggle button (e.g. Mute): tinted when on.
pub fn pill_toggle(on: bool, color_of: fn(&Tokens) -> Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let c = color_of(&t);
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let disabled = status == button::Status::Disabled;
        button::Style {
            background: Some(Background::Color(if on {
                alpha(c, if hover { 0.24 } else { 0.16 })
            } else if hover {
                t.surface_hover
            } else {
                Color::TRANSPARENT
            })),
            text_color: if on {
                c
            } else if disabled {
                alpha(t.muted, 0.5)
            } else {
                t.muted
            },
            border: Border { color: if on { alpha(c, 0.45) } else { t.border }, width: 1.0, radius: 999.0.into() },
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

/// Segment of the Front / Back control.
pub fn segment(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: selected.then_some(Background::Gradient(iced::Gradient::Linear(
                iced::gradient::Linear::new(std::f32::consts::PI)
                    .add_stop(0.0, shade(t.accent, 0.14))
                    .add_stop(1.0, t.accent),
            ))),
            text_color: if selected { t.on_accent } else if hover { t.text } else { t.muted },
            border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 999.0.into() },
            shadow: if selected {
                Shadow { color: alpha(t.accent, 0.35), offset: Vector::new(0.0, 2.0), blur_radius: 6.0 }
            } else {
                Shadow::default()
            },
            snap: true,
        }
    }
}

pub fn segmented_track(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(t.surface)),
        border: Border { color: t.border, width: 1.0, radius: 999.0.into() },
        ..Default::default()
    }
}

/// Window background: a soft vertical wash instead of a flat fill.
pub fn backdrop(t: &Tokens) -> Background {
    let (top, bottom) = if t.dark { (hex(0x1C1F29), hex(0x12141A)) } else { (hex(0xFBFCFF), hex(0xECEFF7)) };
    let g = iced::gradient::Linear::new(std::f32::consts::PI).add_stop(0.0, top).add_stop(1.0, bottom);
    Background::Gradient(iced::Gradient::Linear(g))
}

/// Mix a color toward white (`f` > 0) or black (`f` < 0).
pub fn shade(c: Color, f: f32) -> Color {
    let to = if f >= 0.0 { 1.0 } else { 0.0 };
    let f = f.abs().min(1.0);
    Color { r: c.r + (to - c.r) * f, g: c.g + (to - c.g) * f, b: c.b + (to - c.b) * f, a: c.a }
}

/// A small key cap ("F", "L" ...) for the shortcut hints.
pub fn keycap(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(if t.dark { t.surface_hover } else { Color::WHITE })),
        text_color: Some(t.text),
        border: Border { color: t.border, width: 1.0, radius: 6.0.into() },
        shadow: Shadow { color: alpha(Color::BLACK, if t.dark { 0.45 } else { 0.10 }), offset: Vector::new(0.0, 1.5), blur_radius: 0.0 },
        ..Default::default()
    }
}

/// A clickable status chip in the header (audio settings summary).
pub fn chip(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let base = if t.dark { t.surface } else { Color::WHITE };
        button::Style {
            background: Some(Background::Color(if hover || active { alpha(t.accent, if t.dark { 0.18 } else { 0.10 }) } else { base })),
            text_color: t.text,
            border: Border { color: if hover || active { alpha(t.accent, 0.6) } else { t.border }, width: 1.0, radius: 999.0.into() },
            shadow: Shadow { color: alpha(Color::BLACK, if t.dark { 0.25 } else { 0.06 }), offset: Vector::new(0.0, 1.0), blur_radius: 3.0 },
            snap: true,
        }
    }
}

/// Drop-down field of a pick list.
pub fn pick(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let t = tokens(theme);
    let hot = !matches!(status, pick_list::Status::Active);
    pick_list::Style {
        text_color: t.text,
        placeholder_color: t.muted,
        handle_color: if hot { t.accent } else { t.muted },
        background: Background::Color(if t.dark { t.surface_hover } else { Color::WHITE }),
        border: Border { color: if hot { t.accent } else { t.border }, width: 1.0, radius: 10.0.into() },
    }
}

/// The list that opens from a drop-down.
pub fn pick_menu(theme: &Theme) -> menu::Style {
    let t = tokens(theme);
    menu::Style {
        background: Background::Color(if t.dark { t.surface_hover } else { Color::WHITE }),
        border: Border { color: t.border, width: 1.0, radius: 10.0.into() },
        text_color: t.text,
        selected_text_color: t.on_accent,
        selected_background: Background::Color(t.accent),
        shadow: Shadow { color: alpha(Color::BLACK, 0.20), offset: Vector::new(0.0, 6.0), blur_radius: 18.0 },
    }
}

/// Window-control button (minimize / maximize / close): round, close turns red.
pub fn chrome_button(close: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then_some(Background::Color(if close { t.bad } else { t.surface_hover })),
            text_color: t.text,
            border: border::rounded(999),
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

/// Tooltip bubble.
pub fn tooltip_box(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(Background::Color(if t.dark { t.surface_hover } else { t.bg })),
        text_color: Some(t.text),
        border: Border { color: t.border, width: 1.0, radius: 10.0.into() },
        shadow: Shadow { color: alpha(Color::BLACK, 0.16), offset: Vector::new(0.0, 4.0), blur_radius: 14.0 },
        snap: true,
    }
}
