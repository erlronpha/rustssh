use iced::widget::{button, container, text, text_input};
use iced::{Background, Border, Color, Element, Theme};

use crate::term::Rgb;

use super::Message;

pub(crate) const APP_BG: Color = Color::from_rgb8(11, 13, 16);
pub(crate) const SIDEBAR: Color = Color::from_rgb8(24, 27, 34);
pub(crate) const RAIL: Color = Color::from_rgb8(17, 19, 24);
pub(crate) const CARD: Color = Color::from_rgb8(34, 39, 50);
pub(crate) const LINE: Color = Color::from_rgb8(46, 53, 66);
pub(crate) const TEXT: Color = Color::from_rgb8(232, 236, 242);
pub(crate) const MUTED: Color = Color::from_rgb8(138, 148, 164);
pub(crate) const ACCENT: Color = Color::from_rgb8(56, 148, 236);
pub(crate) const ACCENT_HOVER: Color = Color::from_rgb8(70, 158, 236);
pub(crate) const ACCENT_PRESS: Color = Color::from_rgb8(30, 110, 190);
pub(crate) const OK: Color = Color::from_rgb8(62, 186, 140);
pub(crate) const DANGER: Color = Color::from_rgb8(224, 96, 102);
pub(crate) const WARN_BG: Color = Color::from_rgb8(58, 36, 28);

pub(crate) fn accent_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(13))
        .on_press(message)
        .padding([7, 12])
        .style(primary_style)
        .into()
}

pub(crate) fn tool_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(12).color(TEXT))
        .on_press(message)
        .padding([4, 7])
        .style(ghost_style)
        .into()
}

pub(crate) fn ghost_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(13).color(TEXT))
        .on_press(message)
        .padding([7, 12])
        .style(ghost_style)
        .into()
}

pub(crate) fn danger_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(13))
        .on_press(message)
        .padding([7, 12])
        .style(danger_style)
        .into()
}

pub(crate) fn panel(color: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(color)),
        border: Border {
            color: LINE,
            width: 1.0,
            radius: 0.0.into(),
        },
        text_color: Some(TEXT),
        ..Default::default()
    }
}

pub(crate) fn field_style(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let border = match status {
        text_input::Status::Focused { .. } => ACCENT,
        text_input::Status::Hovered => Color::from_rgb8(78, 90, 110),
        _ => LINE,
    };
    text_input::Style {
        background: Background::Color(APP_BG),
        border: Border {
            color: border,
            width: 1.0,
            radius: 8.0.into(),
        },
        icon: MUTED,
        placeholder: MUTED,
        value: TEXT,
        selection: ACCENT,
    }
}

pub(crate) fn primary_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => ACCENT_HOVER,
        button::Status::Pressed => ACCENT_PRESS,
        button::Status::Disabled => CARD,
        button::Status::Active => ACCENT,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub(crate) fn ghost_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => CARD,
        button::Status::Pressed => LINE,
        _ => Color::from_rgba8(0, 0, 0, 0.0),
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: TEXT,
        border: Border {
            color: LINE,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

pub(crate) fn danger_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb8(196, 72, 78),
        button::Status::Pressed => Color::from_rgb8(150, 48, 54),
        _ => DANGER,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub(crate) fn tab_chip(active: bool) -> container::Style {
    container::Style {
        background: Some(Background::Color(if active {
            CARD
        } else {
            Color::from_rgba8(0, 0, 0, 0.0)
        })),
        border: Border {
            color: if active { LINE } else { Color::from_rgba8(0, 0, 0, 0.0) },
            width: if active { 1.0 } else { 0.0 },
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

pub(crate) fn close_style(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Hovered => (Color::from_rgba8(224, 96, 102, 0.22), DANGER),
        button::Status::Pressed => (Color::from_rgba8(224, 96, 102, 0.38), DANGER),
        _ => (Color::from_rgba8(0, 0, 0, 0.0), MUTED),
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border {
            radius: 6.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub(crate) fn rgb(color: Rgb) -> Color {
    Color::from_rgb8(color.r, color.g, color.b)
}
