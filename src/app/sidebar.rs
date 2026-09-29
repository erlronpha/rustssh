use std::collections::BTreeMap;

use iced::alignment::Vertical;
use iced::widget::{button, column, container, mouse_area, row, scrollable, space, text, text_input};
use iced::{Background, Border, Color, Element, Fill, Point, Task};

use crate::model::Profile;

use super::*;

pub(crate) fn sidebar(app: &App) -> Element<'_, Message> {
    let list = grouped(app);
    let mut sessions = column![].spacing(4).width(Fill);
    if list.is_empty() {
        sessions = sessions.push(
            text(if app.profiles.is_empty() {
                "还没有保存的会话"
            } else {
                "没有匹配的会话"
            })
            .size(13)
            .color(MUTED),
        );
    }
    let dragging = app.session_drag.as_ref().is_some_and(|drag| drag.dragging);
    let hover = app.session_drag.as_ref().and_then(|drag| drag.hover.clone());
    let grabbed = app.session_drag.as_ref().map(|drag| drag.id.clone());
    for (group, profiles) in list {
        let collapsed = app.collapsed.contains(&group);
        let marker = if collapsed { "▸" } else { "▾" };
        let drop_here = dragging && hover.as_deref() == Some(group.as_str());
        let label = if drop_here {
            format!("{marker}  放到 {group}")
        } else {
            format!("{marker}  {group}")
        };
        let enter = group.clone();
        let exit = group.clone();
        sessions = sessions.push(
            mouse_area(
                container(text(label).size(12).color(if drop_here { ACCENT } else { MUTED }))
                    .width(Fill)
                    .padding([6, 4])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(if drop_here {
                            Color::from_rgba8(56, 148, 236, 0.16)
                        } else {
                            Color::from_rgba8(0, 0, 0, 0.0)
                        })),
                        border: Border {
                            radius: 6.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            )
            .on_press(Message::ToggleGroup(group.clone()))
            .on_enter(Message::SessionHoverEnter(enter))
            .on_exit(Message::SessionHoverExit(exit)),
        );
        if collapsed {
            continue;
        }
        for profile in profiles {
            sessions = sessions.push(session_row(
                profile,
                app.selected.as_deref() == Some(&profile.id),
                grabbed.as_deref() == Some(profile.id.as_str()) && dragging,
                app.session_drag.is_some(),
            ));
        }
    }

    container(
        column![
            row![
                text("会话").size(16),
                space::horizontal(),
                ghost_button("清除密钥", Message::ClearHostKey),
            ]
            .align_y(Vertical::Center),
            text_input("搜索名称、主机、用户", &app.search)
                .on_input(Message::Search)
                .padding(8)
                .width(Fill)
                .style(field_style),
            scrollable(sessions).height(Fill).width(Fill),
            row![
                accent_button("连接", Message::ConnectSelected),
                ghost_button("编辑", Message::EditSession),
                button(text("删除").size(13).color(DANGER))
                    .on_press(Message::AskDeleteSession)
                    .padding([7, 12])
                    .style(ghost_style),
            ]
            .spacing(8),
        ]
        .spacing(12)
        .height(Fill),
    )
    .width(app.sidebar_width)
    .height(Fill)
    .padding([14, 14])
    .style(|_| container::Style {
        background: Some(Background::Color(SIDEBAR)),
        text_color: Some(TEXT),
        ..Default::default()
    })
    .into()
}

pub(crate) fn sidebar_splitter() -> Element<'static, Message> {
    mouse_area(
        row![
            container(column![])
                .width(6)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Background::Color(SIDEBAR)),
                    ..Default::default()
                }),
            container(column![])
                .width(2)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Background::Color(LINE)),
                    ..Default::default()
                }),
        ]
        .width(8)
        .height(Fill),
    )
    .on_press(Message::SidebarDragStart)
    .interaction(iced::mouse::Interaction::ResizingHorizontally)
    .into()
}

pub(crate) fn sidebar_drag_move(app: &mut App, x: f32) -> Task<Message> {
    let Some(drag) = app.sidebar_drag else {
        return Task::none();
    };
    let anchor_x = drag.anchor_x.unwrap_or(x);
    app.sidebar_width = (drag.anchor_width + x - anchor_x).clamp(200.0, 640.0);
    app.sidebar_drag = Some(SidebarDrag {
        anchor_x: Some(anchor_x),
        anchor_width: drag.anchor_width,
    });
    Task::none()
}

pub(crate) fn session_label(profile: &Profile) -> String {
    let target = format!("{}@{}:{}", profile.username, profile.host, profile.port);
    let name = profile.name.trim();
    if name.is_empty() || name == profile.host || name == format!("{}:{}", profile.host, profile.port) {
        target
    } else {
        format!("{name}  {target}")
    }
}

pub(crate) fn session_drag_move(app: &mut App, x: f32, y: f32) -> Task<Message> {
    let Some(drag) = app.session_drag.as_mut() else {
        return Task::none();
    };
    let origin = drag.origin.unwrap_or(Point::new(x, y));
    let moved = (x - origin.x).hypot(y - origin.y) > 6.0;
    drag.origin = Some(origin);
    if moved {
        drag.dragging = true;
    }
    Task::none()
}

pub(crate) fn finish_session_drag(app: &mut App) {
    let Some(drag) = app.session_drag.take() else {
        return;
    };
    if !drag.dragging {
        return;
    }
    let Some(label) = drag.hover else {
        return;
    };
    let group = if label == "未分组" {
        String::new()
    } else {
        label.clone()
    };
    let changed = app.profiles.iter_mut().any(|profile| {
        if profile.id != drag.id || profile.group == group {
            return false;
        }
        profile.group = group.clone();
        true
    });
    if !changed {
        return;
    }
    app.collapsed.remove(&label);
    app.status = match model::save_profiles(&app.profiles) {
        Ok(()) => format!("已移动到 {label}"),
        Err(error) => error,
    };
}

pub(crate) fn session_row(profile: &Profile, selected: bool, dragging: bool, held: bool) -> Element<'_, Message> {
    let id = profile.id.clone();
    let open_id = profile.id.clone();
    mouse_area(
        container(
            row![
                container(column![])
                    .width(3)
                    .height(16)
                    .style(move |_| container::Style {
                        background: Some(Background::Color(if selected {
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
                text(session_label(profile)).size(13).color(if selected { TEXT } else { MUTED }),
            ]
            .spacing(10)
            .align_y(Vertical::Center),
        )
        .width(Fill)
        .padding([6, 8])
        .style(move |_| container::Style {
            background: Some(Background::Color(if dragging || selected {
                CARD
            } else {
                Color::from_rgba8(0, 0, 0, 0.0)
            })),
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    )
    .on_press(Message::SessionGrab(id))
    .on_double_click(Message::ConnectProfile(open_id))
    .interaction(if held {
        iced::mouse::Interaction::AllScroll
    } else {
        iced::mouse::Interaction::None
    })
    .into()
}

pub(crate) fn grouped(app: &App) -> Vec<(String, Vec<&Profile>)> {
    let mut groups: BTreeMap<String, Vec<&Profile>> = BTreeMap::new();
    for profile in &app.profiles {
        if !matches_search(profile, &app.search) {
            continue;
        }
        let group = if profile.group.trim().is_empty() {
            "未分组".to_string()
        } else {
            profile.group.clone()
        };
        groups.entry(group).or_default().push(profile);
    }
    for profiles in groups.values_mut() {
        profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }
    groups.into_iter().collect()
}

pub(crate) fn matches_search(profile: &Profile, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    format!(
        "{} {} {} {}",
        profile.name, profile.host, profile.username, profile.group
    )
    .to_lowercase()
    .contains(&query)
}
