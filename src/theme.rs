//! Colour tokens and widget styles. Every surface colour is derived from the
//! five palette colours, so a new theme only needs its three base colours.

use iced::widget::{button, container, pick_list, rule, text, text_editor, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector, font, theme::Palette};
use serde::{Deserialize, Serialize};

pub const FAMILY: &str = "IBM Plex Sans";
pub const REGULAR: Font = Font::with_name(FAMILY);
pub const MEDIUM: Font = Font { weight: font::Weight::Medium, ..REGULAR };
pub const SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..REGULAR };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AppTheme {
    #[default]
    Dark,
    Light,
    CherryBlossom,
    XboxGreen,
    MatteBlack,
    Midnight,
    Ember,
    Parchment,
}

impl AppTheme {
    pub const ALL: [AppTheme; 8] = [
        AppTheme::Dark,
        AppTheme::Light,
        AppTheme::CherryBlossom,
        AppTheme::XboxGreen,
        AppTheme::MatteBlack,
        AppTheme::Midnight,
        AppTheme::Ember,
        AppTheme::Parchment,
    ];

    pub fn name(self) -> &'static str {
        match self {
            AppTheme::Dark => "Dark",
            AppTheme::Light => "Light",
            AppTheme::CherryBlossom => "Cherry Blossom",
            AppTheme::XboxGreen => "Xbox Green",
            AppTheme::MatteBlack => "Matte Black",
            AppTheme::Midnight => "Midnight",
            AppTheme::Ember => "Ember",
            AppTheme::Parchment => "Parchment",
        }
    }

    /// (background, text, accent)
    fn colors(self) -> (u32, u32, u32) {
        match self {
            AppTheme::Dark => (0x131517, 0xE6E8EB, 0x35BBA9),
            AppTheme::Light => (0xF5F4F0, 0x1C1E21, 0x0A675C),
            AppTheme::CherryBlossom => (0xFBEEF3, 0x3A1F2B, 0xA52660),
            AppTheme::XboxGreen => (0x0D130D, 0xE7F0E5, 0x5DC21E),
            AppTheme::MatteBlack => (0x0B0B0C, 0xECECEC, 0xD4D4D4),
            AppTheme::Midnight => (0x0E1626, 0xE3E9F5, 0x7DB0FF),
            AppTheme::Ember => (0x1A1310, 0xF2E6DC, 0xFF8F52),
            AppTheme::Parchment => (0xF3EAD7, 0x33291C, 0x85540F),
        }
    }

    pub fn build(self) -> Theme {
        let (background, text, primary) = self.colors();
        let background = hex(background);
        let dark = luminance(background) < 0.5;
        let palette = Palette {
            background,
            text: hex(text),
            primary: hex(primary),
            success: if dark { hex(0x5BBF86) } else { hex(0x1E7A47) },
            danger: if dark { hex(0xEF6B63) } else { hex(0xB3261E) },
        };
        Theme::custom(self.name().to_string(), palette)
    }
}

/// Derived colours used across the UI.
pub struct Tokens {
    pub bg: Color,
    pub sidebar: Color,
    pub surface: Color,
    pub surface_hover: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub on_accent: Color,
    pub danger: Color,
    pub dark: bool,
}

pub fn tokens(theme: &Theme) -> Tokens {
    let p = theme.palette();
    let dark = luminance(p.background) < 0.5;
    let toward_text = |amount: f32| mix(p.background, p.text, amount);
    Tokens {
        bg: p.background,
        sidebar: if dark { mix(p.background, Color::BLACK, 0.25) } else { mix(p.background, p.text, 0.035) },
        surface: if dark { toward_text(0.045) } else { mix(p.background, Color::WHITE, 0.7) },
        surface_hover: toward_text(if dark { 0.08 } else { 0.05 }),
        border: toward_text(if dark { 0.12 } else { 0.13 }),
        text: p.text,
        muted: toward_text(if dark { 0.62 } else { 0.76 }),
        accent: p.primary,
        accent_soft: mix(p.background, p.primary, if dark { 0.16 } else { 0.12 }),
        on_accent: if contrast(p.primary, hex(0x111315)) > contrast(p.primary, Color::WHITE) { hex(0x111315) } else { Color::WHITE },
        danger: p.danger,
        dark,
    }
}

pub fn app_bg(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style { background: Some(t.bg.into()), text_color: Some(t.text), ..Default::default() }
}

pub fn sidebar(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(t.sidebar.into()),
        border: Border { color: t.border, width: 0.0, radius: 0.0.into() },
        ..Default::default()
    }
}

/// A content panel: one level of elevation, never nested.
pub fn panel(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(t.surface.into()),
        border: Border { color: t.border, width: 1.0, radius: 10.0.into() },
        shadow: if t.dark {
            Shadow::default()
        } else {
            Shadow { color: Color { a: 0.06, ..Color::BLACK }, offset: Vector::new(0.0, 2.0), blur_radius: 8.0 }
        },
        ..Default::default()
    }
}

pub fn banner_error(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(mix(t.bg, t.danger, 0.14).into()),
        text_color: Some(if t.dark { mix(t.danger, Color::WHITE, 0.45) } else { mix(t.danger, Color::BLACK, 0.2) }),
        border: Border { color: mix(t.bg, t.danger, 0.35), width: 1.0, radius: 8.0.into() },
        ..Default::default()
    }
}

pub fn banner_info(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(t.accent_soft.into()),
        text_color: Some(t.text),
        border: Border { color: mix(t.bg, t.accent, 0.35), width: 1.0, radius: 8.0.into() },
        ..Default::default()
    }
}

pub fn table_header(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(t.surface_hover.into()),
        text_color: Some(t.muted),
        border: Border { radius: 6.0.into(), ..Default::default() },
        ..Default::default()
    }
}

pub fn table_row(stripe: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let t = tokens(theme);
        container::Style {
            background: stripe.then(|| Background::Color(mix(t.surface, t.text, 0.025))),
            border: Border { radius: 6.0.into(), ..Default::default() },
            ..Default::default()
        }
    }
}

pub fn primary_button(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    let bg = match status {
        button::Status::Hovered => mix(t.accent, t.text, 0.12),
        button::Status::Pressed => mix(t.accent, t.bg, 0.15),
        button::Status::Disabled => mix(t.bg, t.accent, 0.35),
        button::Status::Active => t.accent,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: t.on_accent,
        border: Border { radius: 8.0.into(), ..Default::default() },
        shadow: Shadow::default(),
    }
}

pub fn secondary_button(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => t.surface_hover,
        _ => t.surface,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: if status == button::Status::Disabled { t.muted } else { t.text },
        border: Border { color: t.border, width: 1.0, radius: 8.0.into() },
        shadow: Shadow::default(),
    }
}

pub fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed).then(|| t.surface_hover.into()),
        text_color: t.text,
        border: Border { radius: 6.0.into(), ..Default::default() },
        shadow: Shadow::default(),
    }
}

pub fn link_button(theme: &Theme, status: button::Status) -> button::Style {
    let t = tokens(theme);
    button::Style {
        background: None,
        text_color: if status == button::Status::Hovered { mix(t.accent, t.text, 0.25) } else { t.accent },
        border: Border::default(),
        shadow: Shadow::default(),
    }
}

pub fn nav_item(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let bg = if active {
            Some(t.accent_soft.into())
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            Some(mix(t.sidebar, t.text, 0.06).into())
        } else {
            None
        };
        button::Style {
            background: bg,
            text_color: if active { t.accent } else { mix(t.sidebar, t.text, 0.82) },
            border: Border { radius: 7.0.into(), ..Default::default() },
            shadow: Shadow::default(),
        }
    }
}

/// A full-width clickable row, e.g. a message in the inbox.
pub fn list_row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let bg = if selected {
            Some(t.accent_soft.into())
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            Some(t.surface_hover.into())
        } else {
            None
        };
        button::Style { background: bg, text_color: t.text, border: Border { radius: 7.0.into(), ..Default::default() }, shadow: Shadow::default() }
    }
}

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let t = tokens(theme);
    let border = match status {
        text_input::Status::Focused => t.accent,
        text_input::Status::Hovered => mix(t.border, t.text, 0.2),
        _ => t.border,
    };
    text_input::Style {
        background: (if t.dark { mix(t.bg, Color::BLACK, 0.2) } else { Color::WHITE }).into(),
        border: Border { color: border, width: 1.0, radius: 8.0.into() },
        icon: t.muted,
        placeholder: t.muted,
        value: t.text,
        selection: mix(t.bg, t.accent, 0.4),
    }
}

/// An editable text area, framed like a text input.
pub fn editor(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let t = tokens(theme);
    let border = match status {
        text_editor::Status::Focused => t.accent,
        text_editor::Status::Hovered => mix(t.border, t.text, 0.2),
        _ => t.border,
    };
    text_editor::Style {
        background: (if t.dark { mix(t.bg, Color::BLACK, 0.2) } else { Color::WHITE }).into(),
        border: Border { color: border, width: 1.0, radius: 8.0.into() },
        icon: t.muted,
        placeholder: t.muted,
        value: t.text,
        selection: mix(t.bg, t.accent, 0.4),
    }
}

pub fn select(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let t = tokens(theme);
    pick_list::Style {
        text_color: t.text,
        placeholder_color: t.muted,
        handle_color: t.muted,
        background: (if matches!(status, pick_list::Status::Hovered) { t.surface_hover } else { t.surface }).into(),
        border: Border {
            color: if matches!(status, pick_list::Status::Opened) { t.accent } else { t.border },
            width: 1.0,
            radius: 8.0.into(),
        },
    }
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t)
}

/// WCAG contrast ratio between two colours.
fn contrast(a: Color, b: Color) -> f32 {
    let linear = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let lum = |c: Color| 0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b);
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn luminance(c: Color) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

fn hex(v: u32) -> Color {
    Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub fn hairline(theme: &Theme) -> rule::Style {
    let t = tokens(theme);
    rule::Style { color: mix(t.surface, t.text, 0.07), width: 1, radius: 0.0.into(), fill_mode: rule::FillMode::Full }
}

/// The mail body: a text editor that looks like plain text but can be selected.
pub fn reader(theme: &Theme, _status: text_editor::Status) -> text_editor::Style {
    let t = tokens(theme);
    text_editor::Style {
        background: Color::TRANSPARENT.into(),
        border: Border::default(),
        icon: t.muted,
        placeholder: t.muted,
        value: t.text,
        selection: mix(t.bg, t.accent, 0.4),
    }
}

/// An empty slot in the week grid: present, but quiet.
pub fn slot_free(theme: &Theme) -> container::Style {
    let t = tokens(theme);
    container::Style {
        background: Some(mix(t.surface, t.text, 0.02).into()),
        border: Border { color: mix(t.surface, t.text, 0.05), width: 1.0, radius: 8.0.into() },
        ..Default::default()
    }
}

/// A class in the week grid. Lectures carry the accent tint; `live` outlines
/// the slot that is on now (or next) today.
pub fn slot_class(lecture: bool, live: bool) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let t = tokens(theme);
        container::Style {
            background: Some((if lecture { t.accent_soft } else { t.surface_hover }).into()),
            text_color: Some(t.text),
            border: Border {
                color: if live { t.accent } else { Color::TRANSPARENT },
                width: if live { 1.5 } else { 0.0 },
                radius: 8.0.into(),
            },
            ..Default::default()
        }
    }
}

/// A readable colour for a GUC letter grade on this theme's panels, from green
/// for an A to red for an F. Anything else (e.g. a pass) keeps the normal text colour.
pub fn grade_color(letter: &str, theme: &Theme) -> Color {
    let t = tokens(theme);
    let pick = |dark: u32, light: u32| hex(if t.dark { dark } else { light });
    match letter.trim().chars().next().map(|c| c.to_ascii_uppercase()) {
        Some('A') => pick(0x5BC98A, 0x17693A),
        Some('B') => pick(0x63B3F2, 0x1A5FA8),
        Some('C') => pick(0xE8C14A, 0x7E5A00),
        Some('D') => pick(0xF2944F, 0xA8480A),
        Some('F') => pick(0xF0726A, 0xB3261E),
        _ => t.text,
    }
}

pub fn grade_text(letter: &str) -> impl Fn(&Theme) -> text::Style + use<> {
    let letter = letter.to_string();
    move |theme| text::Style { color: Some(grade_color(&letter, theme)) }
}

/// A theme's card in the Settings picker.
pub fn theme_card(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let t = tokens(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: (selected || hovered).then(|| (if selected { t.accent_soft } else { t.surface_hover }).into()),
            text_color: t.text,
            border: Border { color: if selected { t.accent } else { Color::TRANSPARENT }, width: 1.5, radius: 10.0.into() },
            shadow: Shadow::default(),
        }
    }
}

/// A flat block of colour, for drawing theme previews.
pub fn swatch(color: Color, radius: f32, border: Option<Color>) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        border: Border { color: border.unwrap_or(Color::TRANSPARENT), width: if border.is_some() { 1.0 } else { 0.0 }, radius: radius.into() },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_get_distinct_colours() {
        let theme = AppTheme::Dark.build();
        let colours = ["A+", "B", "C-", "D+", "F"].map(|g| grade_color(g, &theme));
        for (i, a) in colours.iter().enumerate() {
            assert!(colours[i + 1..].iter().all(|b| a != b));
        }
        assert_eq!(grade_color("a-", &theme), grade_color("A", &theme));
        assert_eq!(grade_color("Pass", &theme), tokens(&theme).text);
    }

    #[test]
    fn every_theme_is_readable() {
        let mut failures = Vec::new();
        for choice in AppTheme::ALL {
            let t = tokens(&choice.build());
            for (what, fg, bg) in [
                ("text on background", t.text, t.bg),
                ("text on panel", t.text, t.surface),
                ("muted on background", t.muted, t.bg),
                ("muted on panel", t.muted, t.surface),
                ("muted on table header", t.muted, t.surface_hover),
                ("muted on sidebar", t.muted, t.sidebar),
                ("accent on panel", t.accent, t.surface),
                ("accent on selected item", t.accent, t.accent_soft),
                ("button label", t.on_accent, t.accent),
                ("grade A", grade_color("A-", &choice.build()), t.surface),
                ("grade B", grade_color("B+", &choice.build()), t.surface),
                ("grade C", grade_color("C", &choice.build()), t.surface),
                ("grade D", grade_color("D", &choice.build()), t.surface),
                ("grade F", grade_color("F", &choice.build()), t.surface),
            ] {
                let ratio = contrast(fg, bg);
                if ratio < 4.5 {
                    failures.push(format!("{}: {what} is only {ratio:.2}:1", choice.name()));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("
"));
    }
}
