use std::sync::OnceLock;

use iced::alignment::Vertical;
use iced::widget::svg::{self, Svg};
use iced::widget::{button, column, container, mouse_area, row, space, stack, text, text_input};
use iced::{Background, Border, Color, Element, Fill, Theme};

use super::dialog::dialog_card;
use super::sftp_ui::sftp_panel;
use super::sidebar::{sidebar, sidebar_splitter};
use super::workspace::workspace;
use super::*;

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let main = column![top_bar(app), body(app)].height(Fill);
    let base = container(main)
        .width(Fill)
        .height(Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(APP_BG)),
            text_color: Some(TEXT),
            ..Default::default()
        });
    if let Some(dialog) = &app.dialog {
        stack([
            base.into(),
            mouse_area(
                container(dialog_card(dialog))
                    .width(Fill)
                    .height(Fill)
                    .center_x(Fill)
                    .center_y(Fill)
                    .style(|_| container::Style {
                        background: Some(Background::Color(Color::from_rgba8(6, 8, 12, 0.67))),
                        ..Default::default()
                    }),
            )
            .on_press(Message::Nop)
            .into(),
        ])
        .into()
    } else {
        base.into()
    }
}

pub(crate) fn top_bar(app: &App) -> Element<'_, Message> {
    container(
        column![
            row![
                text("RustSSH").size(16).color(TEXT),
                space::horizontal(),
                text_input("user@host 或 host:port", &app.quick_target)
                    .on_input(Message::QuickTarget)
                    .on_submit(Message::QuickConnect)
                    .padding(8)
                    .width(260)
                    .style(field_style),
                text_input("密码", &app.quick_password)
                    .on_input(Message::QuickPassword)
                    .on_submit(Message::QuickConnect)
                    .secure(true)
                    .padding(8)
                    .width(140)
                    .style(field_style),
                accent_button("连接", Message::QuickConnect),
                ghost_button("新建会话", Message::NewSession),
            ]
            .spacing(8)
            .padding([10, 16])
            .align_y(Vertical::Center),
            hairline(),
        ],
    )
    .style(|_| container::Style {
        background: Some(Background::Color(SIDEBAR)),
        text_color: Some(TEXT),
        ..Default::default()
    })
    .into()
}

pub(crate) fn hairline() -> Element<'static, Message> {
    container(column![])
        .width(Fill)
        .height(1)
        .style(|_| container::Style {
            background: Some(Background::Color(LINE)),
            ..Default::default()
        })
        .into()
}

pub(crate) fn body(app: &App) -> Element<'_, Message> {
    let mut parts = vec![pane_tabs(app.show_sftp)];
    if app.show_sftp {
        parts.push(sftp_panel(app));
    } else if app.sidebar_open {
        parts.push(sidebar(app));
        parts.push(sidebar_splitter());
    }
    parts.push(workspace(app));
    row(parts).height(Fill).into()
}

pub(crate) fn pane_tabs(files: bool) -> Element<'static, Message> {
    container(
        column![
            pane_tab(ssh_icon(), !files, Message::SelectPane(false)),
            pane_tab(files_icon(), files, Message::SelectPane(true)),
            space::vertical(),
        ]
        .spacing(8)
        .padding([8, 4])
        .height(Fill),
    )
    .width(48)
    .height(Fill)
    .style(|_| container::Style {
        background: Some(Background::Color(RAIL)),
        ..Default::default()
    })
    .into()
}

pub(crate) fn ssh_icon() -> svg::Handle {
    static ICON: OnceLock<svg::Handle> = OnceLock::new();
    ICON.get_or_init(|| svg::Handle::from_memory(include_bytes!("../../assets/ssh.svg")))
        .clone()
}

pub(crate) fn files_icon() -> svg::Handle {
    static ICON: OnceLock<svg::Handle> = OnceLock::new();
    ICON.get_or_init(|| svg::Handle::from_memory(include_bytes!("../../assets/files.svg")))
        .clone()
}

pub(crate) fn pane_tab(icon: svg::Handle, active: bool, message: Message) -> Element<'static, Message> {
    let color = if active { ACCENT } else { MUTED };
    let glyph = Svg::new(icon).width(18).height(18).style(move |_: &Theme, _| svg::Style {
        color: Some(color),
    });
    button(
        row![
            container(column![])
                .width(3)
                .height(16)
                .style(move |_| container::Style {
                    background: Some(Background::Color(if active {
                        ACCENT
                    } else {
                        Color::from_rgba8(0, 0, 0, 0.0)
                    })),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            container(glyph).center_x(Fill).center_y(Fill),
        ]
        .align_y(Vertical::Center)
        .height(Fill),
    )
    .on_press(message)
    .padding([0, 4])
    .width(Fill)
    .height(40)
    .style(move |_, status| pane_tab_style(active, status))
    .into()
}

pub(crate) fn pane_tab_style(active: bool, status: button::Status) -> button::Style {
    let background = if active {
        Color::from_rgba8(56, 148, 236, 0.16)
    } else {
        match status {
            button::Status::Hovered => CARD,
            button::Status::Pressed => LINE,
            _ => Color::from_rgba8(0, 0, 0, 0.0),
        }
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: TEXT,
        border: Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
