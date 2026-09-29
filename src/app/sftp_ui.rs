use iced::alignment::Vertical;
use iced::widget::{column, container, mouse_area, progress_bar, row, scrollable, space, text, text_input};
use iced::{Background, Border, Color, Element, Fill, Task, Theme};

use super::*;

pub(crate) fn sftp_panel(app: &App) -> Element<'_, Message> {
    let body = if let Some(tab) = app.active() {
        sftp_body(tab, &app.sftp_scroll)
    } else {
        container(text("打开会话后可以浏览远程文件").size(13).color(MUTED))
            .padding(12)
            .height(Fill)
            .into()
    };
    container(
        column![
            row![
                text("远程文件").size(16).color(TEXT),
                space::horizontal(),
                ghost_button("隐藏", Message::ToggleSftp),
            ]
            .align_y(Vertical::Center)
            .padding([8, 12]),
            body,
        ]
        .height(Fill),
    )
    .width(app.sftp_width)
    .height(Fill)
    .style(|_| panel(SIDEBAR))
    .into()
}

pub(crate) fn sftp_body<'a>(tab: &'a Tab, scroll: &'a SftpScroll) -> Element<'a, Message> {
    let mut files = column![].spacing(2).width(Fill);
    if tab.cwd != "/" {
        files = files.push(entry_row("..", "上级目录", tab.selected_entry.as_deref() == Some("..")));
    }
    for entry in &tab.entries {
        let kind = if entry.is_link {
            "链接".to_string()
        } else if entry.is_dir {
            "目录".to_string()
        } else {
            human_size(entry.size)
        };
        files = files.push(entry_row(
            &entry.name,
            kind,
            tab.selected_entry.as_deref() == Some(&entry.name),
        ));
    }
    column![
        text_input("/path", &tab.path_edit)
            .on_input(Message::PathEdit)
            .on_submit(Message::SubmitPath)
            .padding(8)
            .width(Fill),
        row![
            tool_button("上级", Message::GoUp),
            tool_button("主目录", Message::GoHome),
            tool_button("刷新", Message::RefreshDir),
            tool_button("新建", Message::CreateDir),
            tool_button("上传", Message::Upload),
            tool_button("下载", Message::Download),
            danger_button("删除", Message::AskDeleteRemote),
        ]
        .spacing(4),
        row![
            text_input("新目录", &tab.mkdir_name)
                .on_input(Message::MkdirName)
                .on_submit(Message::CreateDir)
                .padding(8)
                .width(Fill),
            ghost_button("创建", Message::CreateDir),
        ]
        .spacing(6)
        .align_y(Vertical::Center),
        transfer_view(tab.transfer.as_ref()),
        row![sftp_files(files), sftp_rail(scroll)]
            .height(Fill)
            .width(Fill),
        text(if tab.sftp_status.is_empty() {
            "目录会在连接成功后加载"
        } else {
            tab.sftp_status.as_str()
        })
        .size(12)
        .color(MUTED),
    ]
    .spacing(8)
    .padding(iced::Padding::new(12.0).right(0.0))
    .height(Fill)
    .into()
}

pub(crate) fn sftp_files(files: iced::widget::Column<'_, Message>) -> Element<'_, Message> {
    scrollable(files)
        .id(iced::widget::Id::new("sftp-files"))
        .direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::new().width(0).scroller_width(0),
        ))
        .on_scroll(Message::SftpScrolled)
        .height(Fill)
        .width(Fill)
        .into()
}

pub(crate) fn sftp_rail(scroll: &SftpScroll) -> Element<'_, Message> {
    mouse_area(
        container(rail_thumb(scroll))
            .width(8)
            .height(Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(LINE)),
                ..Default::default()
            }),
    )
    .on_press(Message::SftpDragStart)
    .on_scroll(|delta| {
        let y = match delta {
            iced::mouse::ScrollDelta::Lines { y, .. } => -y * 48.0,
            iced::mouse::ScrollDelta::Pixels { y, .. } => -y,
        };
        Message::SftpRailWheel(y)
    })
    .interaction(iced::mouse::Interaction::ResizingHorizontally)
    .into()
}

pub(crate) fn rail_thumb(scroll: &SftpScroll) -> Element<'static, Message> {
    if scroll.content_h <= scroll.view_h + 1.0 || scroll.view_h <= 1.0 {
        return column![].into();
    }
    let thumb_frac = (scroll.view_h / scroll.content_h).clamp(0.12, 1.0);
    let max_offset = (scroll.content_h - scroll.view_h).max(1.0);
    let travel = 1.0 - thumb_frac;
    let top_frac = (scroll.offset_y / max_offset).clamp(0.0, 1.0) * travel;
    let bottom_frac = (travel - top_frac).max(0.0);
    column![
        space::vertical().height(portion((top_frac * 1000.0) as u16)),
        container(column![])
            .width(Fill)
            .height(portion((thumb_frac * 1000.0).max(1.0) as u16))
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgb8(150, 160, 176))),
                border: Border {
                    radius: 4.0.into(),
                    width: 0.0,
                    color: LINE,
                },
                ..Default::default()
            }),
        space::vertical().height(portion((bottom_frac * 1000.0) as u16)),
    ]
    .height(Fill)
    .padding([2, 1])
    .into()
}

pub(crate) fn portion(value: u16) -> iced::Length {
    if value == 0 {
        iced::Length::Fixed(0.0)
    } else {
        iced::Length::FillPortion(value)
    }
}

pub(crate) fn sftp_drag_move(app: &mut App, x: f32, y: f32) -> Task<Message> {
    let Some(mut drag) = app.sftp_drag else {
        return Task::none();
    };
    let (origin_x, origin_y) = match (drag.anchor_x, drag.anchor_y) {
        (Some(origin_x), Some(origin_y)) => (origin_x, origin_y),
        _ => {
            drag.anchor_x = Some(x);
            drag.anchor_y = Some(y);
            drag.last_y = y;
            app.sftp_drag = Some(drag);
            return Task::none();
        }
    };
    let dx = x - origin_x;
    let dy = y - origin_y;
    if drag.mode == RailDrag::Pending {
        if dx.abs() < 4.0 && dy.abs() < 4.0 {
            return Task::none();
        }
        drag.mode = if dx.abs() > dy.abs() {
            RailDrag::Resize
        } else {
            RailDrag::Scroll
        };
        drag.last_y = y;
    }
    if drag.mode == RailDrag::Resize {
        let width = drag.anchor_width + dx;
        if width < 160.0 {
            app.show_sftp = false;
            app.sftp_drag = None;
        } else {
            app.sftp_width = width.min(720.0);
            app.sftp_drag = Some(drag);
        }
        return Task::none();
    }
    let delta = y - drag.last_y;
    drag.last_y = y;
    app.sftp_drag = Some(drag);
    scroll_files(delta * scroll_scale(&app.sftp_scroll))
}

pub(crate) fn scroll_scale(scroll: &SftpScroll) -> f32 {
    let thumb = scroll.view_h * (scroll.view_h / scroll.content_h.max(1.0));
    let travel = (scroll.view_h - thumb).max(1.0);
    let range = (scroll.content_h - scroll.view_h).max(0.0);
    if scroll.view_h <= 1.0 {
        1.0
    } else {
        range / travel
    }
}

pub(crate) fn scroll_files(delta_y: f32) -> Task<Message> {
    if delta_y == 0.0 {
        return Task::none();
    }
    iced::widget::operation::scroll_by(
        iced::widget::Id::new("sftp-files"),
        scrollable::AbsoluteOffset { x: 0.0, y: delta_y },
    )
}

pub(crate) fn entry_row<'a>(name: &'a str, meta: impl Into<String>, selected: bool) -> Element<'a, Message> {
    let select_name = name.to_string();
    let open_name = name.to_string();
    mouse_area(
        container(
            row![
                text(ellipsize(name, 24)).size(13).color(TEXT).width(Fill),
                text(meta.into()).size(11).color(MUTED),
            ]
            .align_y(Vertical::Center),
        )
        .width(Fill)
        .padding([5, 6])
        .style(move |_| container::Style {
            background: Some(Background::Color(if selected { CARD } else { SIDEBAR })),
            border: Border {
                radius: 4.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    )
    .on_press(Message::SelectEntry(select_name))
    .on_double_click(Message::OpenEntry(open_name))
    .into()
}

pub(crate) fn transfer_view(transfer: Option<&ssh::Transfer>) -> Element<'_, Message> {
    let Some(transfer) = transfer else {
        return column![].height(0).into();
    };
    let ratio = if transfer.total == 0 {
        0.0
    } else {
        (transfer.done as f32 / transfer.total as f32).clamp(0.0, 1.0)
    };
    let bar: Element<'_, Message> = if transfer.total > 0 {
        progress_bar(0.0..=1.0, ratio)
            .girth(6)
            .style(|_: &Theme| progress_bar::Style {
                background: Background::Color(LINE),
                bar: Background::Color(ACCENT),
                border: Border {
                    radius: 3.0.into(),
                    width: 0.0,
                    color: LINE,
                },
            })
            .into()
    } else {
        column![].height(0).into()
    };
    column![bar, text(transfer_label(transfer)).size(12).color(TEXT)]
        .spacing(4)
        .into()
}
