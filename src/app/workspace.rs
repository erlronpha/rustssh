use iced::alignment::{Horizontal, Vertical};
use iced::widget::svg::{self, Svg};
use iced::widget::{button, column, container, mouse_area, progress_bar, row, scrollable, space, text};
use iced::{Background, Border, Element, Fill, Theme};

use crate::ssh::{self, LinkState};
use crate::term::DEFAULT_BG;

use super::chrome::ssh_icon;
use super::term_view::term_canvas;
use super::*;

pub(crate) fn workspace(app: &App) -> Element<'_, Message> {
    let content = if let Some(tab) = app.active() {
        column![tab_strip(app), link_banner(tab), terminal_row(app, tab), status_bar(app, Some(tab))]
            .spacing(0)
            .height(Fill)
    } else {
        column![tab_strip(app), empty_state(), status_bar(app, None)]
            .spacing(0)
            .height(Fill)
    };
    container(content).width(Fill).height(Fill).into()
}

pub(crate) fn tab_strip(app: &App) -> Element<'_, Message> {
    let mut tabs = row![].spacing(4).align_y(Vertical::Center);
    for tab in &app.tabs {
        let active = app.active().map(|item| item.id) == Some(tab.id);
        let color = match tab.link {
            LinkState::Online => OK,
            LinkState::Connecting => ACCENT,
            LinkState::Offline(_) => DANGER,
        };
        let tab_id = tab.id;
        tabs = tabs.push(
            container(
                row![
                    mouse_area(
                        row![
                            text(match tab.link {
                                LinkState::Online => "●",
                                LinkState::Connecting => "◌",
                                LinkState::Offline(_) => "○",
                            })
                            .size(9)
                            .color(color),
                            text(ellipsize(&tab.display_title(), 22)).size(13).color(TEXT),
                        ]
                        .spacing(8)
                        .align_y(Vertical::Center),
                    )
                    .on_press(Message::SelectTab(tab_id)),
                    button(text("×").size(13))
                        .on_press(Message::CloseTab(tab_id))
                        .padding([2, 7])
                        .style(close_style),
                ]
                .spacing(2)
                .align_y(Vertical::Center),
            )
            .padding(iced::Padding {
                top: 3.0,
                right: 4.0,
                bottom: 3.0,
                left: 10.0,
            })
            .style(move |_| tab_chip(active)),
        );
    }
    container(
        row![
            scrollable(tabs)
                .direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::default(),
                ))
                .width(Fill),
            ghost_button(if app.show_sftp { "隐藏文件" } else { "文件" }, Message::ToggleSftp),
        ]
        .spacing(8)
        .align_y(Vertical::Center),
    )
    .padding([8, 10])
    .style(|_| panel(SIDEBAR))
    .into()
}

pub(crate) fn link_banner(tab: &Tab) -> Element<'_, Message> {
    match &tab.link {
        LinkState::Online => container(column![]).height(0).into(),
        LinkState::Connecting => banner(
            format!(
                "正在连接 {}@{}:{}",
                tab.request.username, tab.request.host, tab.request.port
            ),
            row![ghost_button("取消", Message::CloseTab(tab.id))],
        ),
        LinkState::Offline(reason) => {
            let mut actions = row![accent_button("重新连接", Message::Reconnect)].spacing(8);
            if reason.contains("主机密钥") {
                actions = actions.push(ghost_button("清除主机密钥", Message::ClearHostKey));
            }
            banner(reason.clone(), actions)
        }
    }
}

pub(crate) fn banner<'a>(message: String, actions: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(
        row![text(message).size(13).color(TEXT), space::horizontal(), actions.into()]
            .spacing(8)
            .align_y(Vertical::Center),
    )
    .width(Fill)
    .padding([8, 12])
    .style(|_| container::Style {
        background: Some(Background::Color(WARN_BG)),
        ..Default::default()
    })
    .into()
}

pub(crate) fn terminal_row<'a>(app: &'a App, tab: &'a Tab) -> Element<'a, Message> {
    container(term_canvas(tab, app.dialog.is_none()))
        .width(Fill)
        .height(Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(rgb(DEFAULT_BG))),
            ..Default::default()
        })
        .into()
}

pub(crate) fn empty_state() -> Element<'static, Message> {
    let glyph = Svg::new(ssh_icon()).width(28).height(28).style(|_: &Theme, _| svg::Style {
        color: Some(ACCENT),
    });
    container(
        column![
            container(glyph)
                .padding(16)
                .style(|_| container::Style {
                    background: Some(Background::Color(CARD)),
                    border: Border {
                        radius: 16.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            text("还没有打开的会话").size(20).color(TEXT),
            text("双击左侧会话，或在上方输入地址快速连接")
                .size(13)
                .color(MUTED),
        ]
        .spacing(12)
        .align_x(Horizontal::Center),
    )
    .width(Fill)
    .height(Fill)
    .center_x(Fill)
    .center_y(Fill)
    .into()
}

pub(crate) fn status_bar<'a>(app: &'a App, tab: Option<&'a Tab>) -> Element<'a, Message> {
    let left = if let Some(tab) = tab {
        match &tab.link {
            LinkState::Online => format!(
                "已连接  {}@{}:{}   {}×{}",
                tab.request.username,
                tab.request.host,
                tab.request.port,
                tab.term.cols(),
                tab.term.rows()
            ),
            LinkState::Connecting => "正在连接".to_string(),
            LinkState::Offline(reason) => reason.clone(),
        }
    } else {
        app.status.clone()
    };
    let fingerprint = tab
        .map(|tab| short_text(&tab.fingerprint, 32))
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    container(
        column![
            row![
                text(left).size(12).color(MUTED),
                space::horizontal(),
                text(fingerprint).size(12).color(MUTED),
                text("拖拽复制 · 右键粘贴 · Ctrl+Tab")
                    .size(12)
                    .color(MUTED),
            ]
            .align_y(Vertical::Center),
            monitor_bar(tab),
        ]
        .spacing(4),
    )
    .padding([6, 12])
    .style(|_| panel(SIDEBAR))
    .into()
}

pub(crate) fn monitor_bar(tab: Option<&Tab>) -> Element<'_, Message> {
    let Some(tab) = tab else {
        return column![].height(0).into();
    };
    let Some(stats) = &tab.stats else {
        return text(if matches!(tab.link, LinkState::Online) {
            "正在读取远程监控…"
        } else {
            ""
        })
        .size(11)
        .color(MUTED)
        .into();
    };
    let mut chips = row![].spacing(6).align_y(Vertical::Center);
    chips = chips.push(stat_chip(stats.host.clone()));
    chips = chips.push(cpu_chip(stats.cpu_percent));
    chips = chips.push(stat_chip(format!(
        "{} / {}",
        human_size(stats.mem_used),
        human_size(stats.mem_total)
    )));
    chips = chips.push(stat_chip(format!("↑ {}/s", human_size(stats.net_tx))));
    chips = chips.push(stat_chip(format!("↓ {}/s", human_size(stats.net_rx))));
    chips = chips.push(stat_chip(format_uptime(stats.uptime_secs)));
    if stats.sessions > 0 {
        chips = chips.push(stat_chip(format!(
            "{}×{}",
            ellipsize(&tab.request.username, 12),
            stats.sessions
        )));
    }
    for disk in pick_disks(&stats.disks) {
        chips = chips.push(stat_chip(format!("{} {}%", disk.mount, disk.percent)));
    }
    scrollable(chips)
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default(),
        ))
        .width(Fill)
        .into()
}

pub(crate) fn cpu_chip(percent: f32) -> Element<'static, Message> {
    let color = if percent >= 85.0 { DANGER } else { OK };
    row![
        text(format!("CPU {percent:.0}%")).size(11).color(TEXT),
        progress_bar(0.0..=100.0, percent)
            .length(48)
            .girth(6)
            .style(move |_: &Theme| progress_bar::Style {
                background: Background::Color(LINE),
                bar: Background::Color(color),
                border: Border {
                    radius: 3.0.into(),
                    width: 0.0,
                    color: LINE,
                },
            }),
    ]
    .spacing(6)
    .align_y(Vertical::Center)
    .into()
}

pub(crate) fn stat_chip(label: String) -> Element<'static, Message> {
    container(text(label).size(11).color(TEXT))
        .padding([2, 6])
        .style(|_| container::Style {
            background: Some(Background::Color(CARD)),
            border: Border {
                radius: 4.0.into(),
                width: 0.0,
                color: CARD,
            },
            ..Default::default()
        })
        .into()
}

pub(crate) fn pick_disks(disks: &[ssh::DiskUsage]) -> Vec<&ssh::DiskUsage> {
    let mut chosen = Vec::new();
    if let Some(root) = disks.iter().find(|disk| disk.mount == "/") {
        chosen.push(root);
    }
    for disk in disks {
        if chosen.len() >= 3 {
            break;
        }
        if chosen.iter().any(|item| item.mount == disk.mount) {
            continue;
        }
        if disk.mount.starts_with("/boot") || disk.mount.starts_with("/home") {
            chosen.push(disk);
        }
    }
    for disk in disks {
        if chosen.len() >= 3 {
            break;
        }
        if chosen.iter().any(|item| item.mount == disk.mount) {
            continue;
        }
        chosen.push(disk);
    }
    chosen
}

pub(crate) fn format_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    if days >= 1 {
        return format!("{days} 天");
    }
    let hours = secs / 3_600;
    if hours >= 1 {
        return format!("{hours} 小时");
    }
    format!("{} 分钟", (secs / 60).max(1))
}
