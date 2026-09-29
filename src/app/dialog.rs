use iced::alignment::Vertical;
use iced::widget::{checkbox, column, container, pick_list, row, text, text_input};
use iced::{Background, Border, Element, Fill};

use super::*;

pub(crate) const AUTH_KINDS: [AuthKind; 2] = [AuthKind::Password, AuthKind::PrivateKey];

pub(crate) fn dialog_card(dialog: &Dialog) -> Element<'_, Message> {
    match dialog {
        Dialog::Editor(editor) => editor_card(editor),
        Dialog::Password {
            password, remember, ..
        } => card(
            "输入密码",
            column![
                labeled(
                    "密码",
                    text_input("", password)
                        .secure(true)
                        .on_input(Message::PasswordChanged)
                        .on_submit(Message::SubmitPassword)
                        .padding(8)
                        .width(Fill),
                ),
                checkbox(*remember)
                    .label("记住密码，明文保存在本机配置目录")
                    .on_toggle(Message::PasswordRemember),
                row![
                    accent_button("连接", Message::SubmitPassword),
                    ghost_button("取消", Message::CloseDialog),
                ]
                .spacing(8),
            ]
            .spacing(12),
        ),
        Dialog::DeleteSession { name, .. } => card(
            "删除会话",
            column![
                text(format!("删除“{name}”？已打开的终端不会一起关闭。")).color(TEXT),
                row![
                    danger_button("删除", Message::DeleteSession),
                    ghost_button("取消", Message::CloseDialog),
                ]
                .spacing(8),
            ]
            .spacing(12),
        ),
        Dialog::DeleteRemote { path, dir } => card(
            "删除远程文件",
            column![
                text(format!(
                    "删除{} {}？此操作直接作用于服务器。",
                    if *dir { "目录" } else { "文件" },
                    path
                ))
                .color(TEXT),
                row![
                    danger_button("删除", Message::DeleteRemote),
                    ghost_button("取消", Message::CloseDialog),
                ]
                .spacing(8),
            ]
            .spacing(12),
        ),
    }
}

pub(crate) fn editor_card(editor: &Editor) -> Element<'_, Message> {
    let auth_fields: Element<'_, Message> = match editor.auth {
        AuthKind::Password => column![
            labeled(
                "密码",
                text_input("", &editor.password)
                    .secure(true)
                    .on_input(Message::EditorPassword)
                    .padding(8)
                    .width(Fill),
            ),
            checkbox(editor.remember)
                .label("记住密码。明文写入本机配置目录，只适合个人电脑")
                .on_toggle(Message::EditorRemember),
        ]
        .spacing(8)
        .into(),
        AuthKind::PrivateKey => column![
            labeled(
                "私钥路径",
                row![
                    text_input("私钥文件", &editor.key_path)
                        .on_input(Message::EditorKey)
                        .padding(8)
                        .width(Fill),
                    ghost_button("浏览", Message::BrowseKey),
                ]
                .spacing(6)
                .align_y(Vertical::Center),
            ),
            labeled(
                "私钥口令",
                text_input("没有口令可留空", &editor.passphrase)
                    .secure(true)
                    .on_input(Message::EditorPassphrase)
                    .padding(8)
                    .width(Fill),
            ),
        ]
        .spacing(8)
        .into(),
    };
    let mut form = column![
        labeled(
            "名称",
            text_input("留空则使用 用户@主机", &editor.name)
                .on_input(Message::EditorName)
                .padding(8)
                .width(Fill),
        ),
        labeled(
            "分组",
            text_input("例如 生产环境", &editor.group)
                .on_input(Message::EditorGroup)
                .padding(8)
                .width(Fill),
        ),
        row![
            labeled(
                "主机",
                text_input("10.0.0.8", &editor.host)
                    .on_input(Message::EditorHost)
                    .padding(8)
                    .width(Fill),
            ),
            labeled(
                "端口",
                text_input("22", &editor.port)
                    .on_input(Message::EditorPort)
                    .padding(8)
                    .width(90),
            ),
        ]
        .spacing(8),
        labeled(
            "用户名",
            text_input("root", &editor.username)
                .on_input(Message::EditorUser)
                .padding(8)
                .width(Fill),
        ),
        labeled(
            "认证",
            pick_list(AUTH_KINDS, Some(editor.auth), Message::EditorAuth).width(Fill),
        ),
        auth_fields,
    ]
    .spacing(10);
    if !editor.error.is_empty() {
        form = form.push(text(editor.error.clone()).color(DANGER).size(13));
    }
    form = form.push(
        row![
            accent_button("保存", Message::SaveEditor),
            ghost_button("取消", Message::CloseDialog),
        ]
        .spacing(8),
    );
    card(if editor.id.is_some() { "编辑会话" } else { "新建会话" }, form)
}

pub(crate) fn card<'a>(title: &'a str, body: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(
        column![text(title).size(18).color(TEXT), body.into()]
            .spacing(14)
            .width(Fill),
    )
    .width(460)
    .padding(20)
    .style(|_| container::Style {
        background: Some(Background::Color(CARD)),
        border: Border {
            color: LINE,
            width: 1.0,
            radius: 12.0.into(),
        },
        ..Default::default()
    })
    .into()
}

pub(crate) fn labeled<'a>(label: &'a str, field: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![text(label).size(12).color(MUTED), field.into()]
        .spacing(4)
        .width(Fill)
        .into()
}
