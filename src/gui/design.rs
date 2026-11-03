//! Reusable smoked-glass charcoal and lavender design tokens for the desktop UI.

use iced::{
    Background, Color, Shadow, Theme, border,
    theme::Palette,
    widget::{button, container, overlay::menu, pick_list},
};

pub const BACKGROUND: Color = Color::from_rgb8(0x17, 0x15, 0x1F);
pub const SURFACE: Color = Color::from_rgb8(0x21, 0x1E, 0x2C);
pub const SURFACE_ELEVATED: Color = Color::from_rgb8(0x2B, 0x26, 0x38);
pub const LAVENDER: Color = Color::from_rgb8(0xB9, 0xA0, 0xF8);
pub const LAVENDER_SOFT: Color = Color::from_rgb8(0xD0, 0xC0, 0xFF);
pub const LAVENDER_MUTED: Color = Color::from_rgb8(0x8F, 0x7B, 0xBE);
pub const TEXT_PRIMARY: Color = Color::from_rgb8(0xF1, 0xED, 0xF8);
pub const TEXT_SECONDARY: Color = Color::from_rgb8(0xA9, 0xA2, 0xB8);
pub const BORDER_SUBTLE: Color = Color::from_rgba8(255, 255, 255, 24.0 / 255.0);
pub const STATUS_CONNECTED: Color = Color::from_rgb8(0xA6, 0xE3, 0xA1);
pub const STATUS_WARNING: Color = Color::from_rgb8(0xF9, 0xE2, 0xAF);
pub const STATUS_ERROR: Color = Color::from_rgb8(0xF3, 0x8B, 0xA8);

pub mod spacing {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

pub mod radius {
    pub const CONTROL: f32 = 8.0;
    pub const CARD: f32 = 12.0;
    pub const WINDOW: f32 = 16.0;
    pub const PILL: f32 = 999.0;
}

pub mod typography {
    pub const CAPTION: f32 = 11.0;
    pub const BODY: f32 = 13.0;
    pub const BODY_LARGE: f32 = 15.0;
    pub const HEADING: f32 = 20.0;
    pub const TITLE: f32 = 24.0;
}

/// The shared Iced palette. Individual widgets use the explicit tokens below
/// where the palette's six semantic roles are not expressive enough.
pub fn theme() -> Theme {
    Theme::custom(
        "nowplaying",
        Palette {
            background: BACKGROUND,
            text: TEXT_PRIMARY,
            primary: LAVENDER,
            success: STATUS_CONNECTED,
            warning: STATUS_WARNING,
            danger: STATUS_ERROR,
        },
    )
}

pub fn window_surface() -> container::Style {
    container::Style {
        background: Some(Background::Color(BACKGROUND)),
        text_color: Some(TEXT_PRIMARY),
        ..container::Style::default()
    }
}

pub fn surface() -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        text_color: Some(TEXT_PRIMARY),
        border: border::rounded(radius::CARD)
            .color(BORDER_SUBTLE)
            .width(1.0),
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

pub fn elevated_surface() -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE_ELEVATED)),
        text_color: Some(TEXT_PRIMARY),
        border: border::rounded(radius::CONTROL)
            .color(BORDER_SUBTLE)
            .width(1.0),
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

pub fn primary_button(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Active => LAVENDER,
        button::Status::Hovered => LAVENDER_SOFT,
        button::Status::Pressed => LAVENDER_MUTED,
        button::Status::Disabled => SURFACE_ELEVATED,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: if status == button::Status::Disabled {
            TEXT_SECONDARY
        } else {
            BACKGROUND
        },
        border: border::rounded(radius::CONTROL),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn secondary_button(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Active | button::Status::Disabled => SURFACE_ELEVATED,
        button::Status::Hovered => SURFACE,
        button::Status::Pressed => BACKGROUND,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: if status == button::Status::Disabled {
            TEXT_SECONDARY
        } else {
            LAVENDER_SOFT
        },
        border: border::rounded(radius::CONTROL)
            .color(BORDER_SUBTLE)
            .width(1.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn danger_button(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Active => STATUS_ERROR,
        button::Status::Hovered => Color::from_rgb8(0xF5, 0xA0, 0xB6),
        button::Status::Pressed => Color::from_rgb8(0xD9, 0x6F, 0x8B),
        button::Status::Disabled => SURFACE_ELEVATED,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: BACKGROUND,
        border: border::rounded(radius::CONTROL),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn navigation_button(_theme: &Theme, _status: button::Status, selected: bool) -> button::Style {
    let background = if selected { SURFACE_ELEVATED } else { SURFACE };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: if selected {
            LAVENDER_SOFT
        } else {
            TEXT_SECONDARY
        },
        border: border::rounded(radius::CONTROL)
            .color(if selected {
                LAVENDER_MUTED
            } else {
                BORDER_SUBTLE
            })
            .width(1.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn dropdown(_theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let background = match status {
        pick_list::Status::Active => SURFACE_ELEVATED,
        pick_list::Status::Hovered => SURFACE,
        pick_list::Status::Opened { .. } => BACKGROUND,
    };
    pick_list::Style {
        text_color: TEXT_PRIMARY,
        placeholder_color: TEXT_SECONDARY,
        handle_color: LAVENDER,
        background: Background::Color(background),
        border: border::rounded(radius::CONTROL)
            .color(BORDER_SUBTLE)
            .width(1.0),
    }
}

pub fn dropdown_menu(_theme: &Theme) -> menu::Style {
    menu::Style {
        background: Background::Color(SURFACE_ELEVATED),
        border: border::rounded(radius::CONTROL)
            .color(BORDER_SUBTLE)
            .width(1.0),
        text_color: TEXT_PRIMARY,
        selected_text_color: TEXT_PRIMARY,
        selected_background: Background::Color(LAVENDER_MUTED),
        shadow: Shadow::default(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    Warning,
}

pub const fn connection_color(state: ConnectionState) -> Color {
    match state {
        ConnectionState::Connected => STATUS_CONNECTED,
        ConnectionState::Disconnected => TEXT_SECONDARY,
        ConnectionState::Warning => STATUS_WARNING,
    }
}

pub fn status_indicator(state: ConnectionState) -> container::Style {
    container::Style {
        background: Some(Background::Color(connection_color(state))),
        border: border::rounded(radius::PILL),
        ..container::Style::default()
    }
}
