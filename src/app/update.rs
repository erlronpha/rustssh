use iced::Task;

use super::sidebar::{finish_session_drag, session_drag_move, sidebar_drag_move};
use super::sftp_ui::{scroll_files, sftp_drag_move};
use super::term_view::plain_key;
use super::{App, Message};
use super::*;

pub(super) fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Nop => Task::none(),
        Message::Worker(WorkerOut::Ready(bridge)) => {
            if app.bridge.is_some() {
                for tab in &mut app.tabs {
                    tab.link = LinkState::Offline("连接进程已重置".to_string());
                }
            }
            app.bridge = Some(bridge);
            let queued = std::mem::take(&mut app.queued);
            for message in queued {
                app.send(message);
            }
            Task::none()
        }
        Message::Worker(WorkerOut::Data { tab, gen, bytes }) => {
            if let Some(index) = app.tab_index(tab, Some(gen)) {
                if let Some(title) = app.tabs[index].term.feed(&bytes) {
                    app.tabs[index].remote_title = Some(title);
                }
                app.tabs[index].cache.clear();
            }
            Task::none()
        }
        Message::Worker(WorkerOut::Status { tab, gen, state }) => {
            if let Some(index) = app.tab_index(tab, Some(gen)) {
                if let LinkState::Offline(reason) = &state {
                    app.status = reason.clone();
                }
                app.tabs[index].link = state;
                app.tabs[index].cache.clear();
            }
            Task::none()
        }
        Message::Worker(WorkerOut::HostKey {
            tab,
            gen,
            fingerprint,
        }) => {
            if let Some(index) = app.tab_index(tab, Some(gen)) {
                let host = app.tabs[index].request.host.clone();
                let port = app.tabs[index].request.port;
                app.tabs[index].fingerprint = fingerprint.clone();
                app.tabs[index].request.known_fingerprint = Some(fingerprint.clone());
                app.known.insert(model::host_key(&host, port), fingerprint);
                if let Err(error) = model::save_known_hosts(&app.known) {
                    app.status = error;
                }
            }
            Task::none()
        }
        Message::Worker(WorkerOut::Dir {
            tab,
            gen,
            path,
            entries,
        }) => {
            if let Some(index) = app.tab_index(tab, Some(gen)) {
                if app.tabs[index].home.is_empty() {
                    app.tabs[index].home = path.clone();
                }
                app.tabs[index].cwd = path.clone();
                app.tabs[index].path_edit = path;
                app.tabs[index].sftp_status = format!("{} 个条目", entries.len());
                app.tabs[index].selected_entry = None;
                app.tabs[index].entries = entries;
            }
            Task::none()
        }
        Message::Worker(WorkerOut::Stats { tab, gen, stats }) => {
            if let Some(index) = app.tab_index(tab, Some(gen)) {
                app.tabs[index].stats = Some(stats);
            }
            Task::none()
        }
        Message::Worker(WorkerOut::Transfer { tab, gen, transfer }) => {
            if let Some(index) = app.tab_index(tab, Some(gen)) {
                let label = transfer_label(&transfer);
                app.tabs[index].transfer = Some(transfer);
                app.status = label;
            }
            Task::none()
        }
        Message::Worker(WorkerOut::Notice {
            tab,
            gen,
            text,
            refresh,
        }) => {
            let relist = if let Some(index) = app.tab_index(tab, Some(gen)) {
                app.tabs[index].transfer = None;
                app.tabs[index].sftp_status = text.clone();
                app.status = text;
                if refresh {
                    Some(app.tabs[index].cwd.clone())
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(path) = relist {
                app.send(WorkerIn::List { tab, path });
            }
            Task::none()
        }
        Message::QuickTarget(value) => {
            app.quick_target = value;
            Task::none()
        }
        Message::QuickPassword(value) => {
            app.quick_password = value;
            Task::none()
        }
        Message::QuickConnect => {
            match model::parse_target(&app.quick_target) {
                Some(target) => {
                    let username = target.user.unwrap_or_else(model::default_user);
                    let known = app
                        .known
                        .get(&model::host_key(&target.host, target.port))
                        .cloned();
                    app.open_tab(
                        target.host.clone(),
                        None,
                        target.host.clone(),
                        target.port,
                        username,
                        ssh::RuntimeAuth::Password(app.quick_password.clone()),
                        known,
                    );
                }
                None => app.status = "无法识别地址，示例：root@10.0.0.8:22".to_string(),
            }
            Task::none()
        }
        Message::Search(value) => {
            app.search = value;
            Task::none()
        }
        Message::ConnectProfile(id) => {
            app.connect_profile(&id, None, false);
            Task::none()
        }
        Message::ConnectSelected => {
            if let Some(id) = app.selected.clone() {
                app.connect_profile(&id, None, false);
            } else {
                app.status = "请先选择一个会话".to_string();
            }
            Task::none()
        }
        Message::NewSession => {
            app.dialog = Some(Dialog::Editor(Editor {
                id: None,
                name: String::new(),
                group: String::new(),
                host: String::new(),
                port: "22".to_string(),
                username: model::default_user(),
                auth: AuthKind::Password,
                password: String::new(),
                remember: false,
                key_path: String::new(),
                passphrase: String::new(),
                error: String::new(),
            }));
            Task::none()
        }
        Message::EditSession => {
            if let Some(profile) = app.selected_profile().cloned() {
                app.dialog = Some(Dialog::Editor(App::editor_from(&profile)));
            } else {
                app.status = "请先选择一个会话".to_string();
            }
            Task::none()
        }
        Message::AskDeleteSession => {
            if let Some(profile) = app.selected_profile() {
                app.dialog = Some(Dialog::DeleteSession {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                });
            } else {
                app.status = "请先选择一个会话".to_string();
            }
            Task::none()
        }
        Message::DeleteSession => {
            if let Some(Dialog::DeleteSession { id, name }) = app.dialog.take() {
                app.profiles.retain(|profile| profile.id != id);
                if app.selected.as_ref() == Some(&id) {
                    app.selected = None;
                }
                if let Err(error) = model::save_profiles(&app.profiles) {
                    app.status = error;
                } else {
                    app.status = format!("已删除会话 {name}");
                }
            }
            Task::none()
        }
        Message::ClearHostKey => {
            app.clear_host_key();
            Task::none()
        }
        Message::SessionGrab(id) => {
            app.selected = Some(id.clone());
            app.sidebar_drag = None;
            app.sftp_drag = None;
            app.session_drag = Some(SessionDrag {
                id,
                origin: None,
                dragging: false,
                hover: None,
            });
            Task::none()
        }
        Message::SessionHoverEnter(group) => {
            if let Some(drag) = &mut app.session_drag {
                if drag.dragging {
                    drag.hover = Some(group.clone());
                    app.collapsed.remove(&group);
                }
            }
            Task::none()
        }
        Message::SessionHoverExit(group) => {
            if let Some(drag) = &mut app.session_drag {
                if drag.hover.as_deref() == Some(group.as_str()) {
                    drag.hover = None;
                }
            }
            Task::none()
        }
        Message::ToggleGroup(group) => {
            if !app.collapsed.insert(group.clone()) {
                app.collapsed.remove(&group);
            }
            Task::none()
        }
        Message::EditorName(value) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.name = value;
            }
            Task::none()
        }
        Message::EditorGroup(value) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.group = value;
            }
            Task::none()
        }
        Message::EditorHost(value) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.host = value;
            }
            Task::none()
        }
        Message::EditorPort(value) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.port = value;
            }
            Task::none()
        }
        Message::EditorUser(value) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.username = value;
            }
            Task::none()
        }
        Message::EditorAuth(kind) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.auth = kind;
            }
            Task::none()
        }
        Message::EditorPassword(value) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.password = value;
            }
            Task::none()
        }
        Message::EditorRemember(value) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.remember = value;
            }
            Task::none()
        }
        Message::EditorKey(value) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.key_path = value;
            }
            Task::none()
        }
        Message::EditorPassphrase(value) => {
            if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                editor.passphrase = value;
            }
            Task::none()
        }
        Message::BrowseKey => Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .set_title("选择私钥")
                    .pick_file()
                    .await
                    .map(|file| file.path().display().to_string())
            },
            Message::KeyPicked,
        ),
        Message::KeyPicked(path) => {
            if let Some(path) = path {
                if let Some(Dialog::Editor(editor)) = &mut app.dialog {
                    editor.key_path = path;
                }
            }
            Task::none()
        }
        Message::SaveEditor => {
            app.save_editor();
            Task::none()
        }
        Message::CloseDialog => {
            app.dialog = None;
            Task::none()
        }
        Message::PasswordChanged(value) => {
            if let Some(Dialog::Password { password, .. }) = &mut app.dialog {
                *password = value;
            }
            Task::none()
        }
        Message::PasswordRemember(value) => {
            if let Some(Dialog::Password { remember, .. }) = &mut app.dialog {
                *remember = value;
            }
            Task::none()
        }
        Message::SubmitPassword => {
            if let Some(Dialog::Password {
                profile_id,
                password,
                remember,
            }) = app.dialog.take()
            {
                app.connect_profile(&profile_id, Some(password), remember);
            }
            Task::none()
        }
        Message::SelectTab(id) => {
            if let Some(index) = app.tab_index(id, None) {
                app.active = Some(index);
                app.sync_pty();
            }
            Task::none()
        }
        Message::CloseTab(id) => {
            app.close_tab(id);
            Task::none()
        }
        Message::CycleTab(delta) => {
            let len = app.tabs.len() as i32;
            if len > 0 {
                let current = app.active.unwrap_or(0) as i32;
                let next = (current + delta).rem_euclid(len) as usize;
                app.active = Some(next);
                app.sync_pty();
            }
            Task::none()
        }
        Message::Reconnect => {
            app.reconnect();
            Task::none()
        }
        Message::ToggleSftp => {
            app.show_sftp = !app.show_sftp;
            app.sftp_drag = None;
            Task::none()
        }
        Message::SelectPane(files) => {
            app.sftp_drag = None;
            if files {
                app.show_sftp = !app.show_sftp;
                if !app.show_sftp {
                    app.sftp_drag = None;
                }
            } else if app.show_sftp {
                app.show_sftp = false;
                app.sidebar_open = true;
            } else {
                app.sidebar_open = !app.sidebar_open;
                app.sidebar_drag = None;
            }
            Task::none()
        }
        Message::SidebarDragStart => {
            app.sftp_drag = None;
            app.sidebar_drag = Some(SidebarDrag {
                anchor_x: None,
                anchor_width: app.sidebar_width,
            });
            Task::none()
        }
        Message::SftpDragStart => {
            app.sidebar_drag = None;
            app.sftp_drag = Some(SftpDrag {
                anchor_x: None,
                anchor_y: None,
                last_y: 0.0,
                anchor_width: app.sftp_width,
                mode: RailDrag::Pending,
            });
            Task::none()
        }
        Message::SftpDragMove(x, y) => {
            if app.session_drag.is_some() {
                session_drag_move(app, x, y)
            } else if app.sidebar_drag.is_some() {
                sidebar_drag_move(app, x)
            } else {
                sftp_drag_move(app, x, y)
            }
        }
        Message::SftpDragEnd => {
            finish_session_drag(app);
            app.sftp_drag = None;
            app.sidebar_drag = None;
            Task::none()
        }
        Message::SftpScrolled(viewport) => {
            let offset = viewport.absolute_offset();
            app.sftp_scroll = SftpScroll {
                offset_y: offset.y,
                view_h: viewport.bounds().height,
                content_h: viewport.content_bounds().height,
            };
            Task::none()
        }
        Message::SftpRailWheel(delta) => scroll_files(delta),
        Message::PaneResized(cols, rows) => {
            app.pane_cols = cols;
            app.pane_rows = rows;
            app.sync_pty();
            Task::none()
        }
        Message::TermKey { tab, input } => {
            let effect = app
                .tabs
                .iter()
                .find(|item| item.id == tab)
                .map(|item| item.term.encode_key(&input));
            match effect {
                Some(InputEffect::Bytes(data)) => {
                    if matches!(input.key, KeyCode::Enter) {
                        if let Some(index) = app.tab_index(tab, None) {
                            if !app.tabs[index].term.is_alt() {
                                app.tabs[index].term.scroll_to_bottom();
                                app.tabs[index].cache.clear();
                            }
                        }
                    }
                    app.send_input(tab, data);
                    Task::none()
                }
                Some(InputEffect::Scroll(delta)) => {
                    if let Some(index) = app.tab_index(tab, None) {
                        app.tabs[index].term.scroll_by(delta);
                        app.tabs[index].cache.clear();
                    }
                    Task::none()
                }
                Some(InputEffect::Copy) => {
                    let text = app
                        .tabs
                        .iter()
                        .find(|item| item.id == tab)
                        .and_then(|item| item.term.selected_text());
                    if let Some(text) = text {
                        iced::clipboard::write(text)
                    } else {
                        Task::none()
                    }
                }
                Some(InputEffect::Paste) => iced::clipboard::read().map(Message::Pasted),
                Some(InputEffect::Ignore) | None => Task::none(),
            }
        }
        Message::Pointer { tab, action } => {
            let Some(index) = app.tab_index(tab, None) else {
                return Task::none();
            };
            match action {
                Pointer::Down(col, row) => {
                    app.tabs[index].selecting = true;
                    app.tabs[index].dragged = false;
                    app.tabs[index].term.start_select(row, col);
                    app.tabs[index].cache.clear();
                    Task::none()
                }
                Pointer::Drag(col, row) => {
                    if app.tabs[index].selecting {
                        app.tabs[index].dragged = true;
                        app.tabs[index].term.extend_select(row, col);
                        app.tabs[index].cache.clear();
                    }
                    Task::none()
                }
                Pointer::Up => {
                    let dragged = app.tabs[index].dragged;
                    app.tabs[index].selecting = false;
                    let text = app.tabs[index].term.finish_select(dragged);
                    app.tabs[index].cache.clear();
                    if let Some(text) = text {
                        iced::clipboard::write(text)
                    } else {
                        Task::none()
                    }
                }
                Pointer::Right => iced::clipboard::read().map(Message::Pasted),
            }
        }
        Message::Wheel { tab, delta } => {
            let Some(index) = app.tab_index(tab, None) else {
                return Task::none();
            };
            if app.tabs[index].term.is_alt() {
                let steps = delta.abs().round().max(1.0) as usize;
                let code = if delta > 0.0 {
                    KeyCode::Up
                } else {
                    KeyCode::Down
                };
                let mut data = Vec::new();
                for _ in 0..steps.min(12) {
                    if let InputEffect::Bytes(part) = app.tabs[index].term.encode_key(&plain_key(code.clone()))
                    {
                        data.extend(part);
                    }
                }
                app.send_input(tab, data);
            } else {
                app.tabs[index].term.scroll_by(delta.round() as i32);
                app.tabs[index].cache.clear();
            }
            Task::none()
        }
        Message::Pasted(text) => {
            if let Some(text) = text {
                let pasted = app
                    .active()
                    .map(|tab| (tab.id, tab.term.paste_bytes(&text)));
                if let Some((id, data)) = pasted {
                    app.send_input(id, data);
                }
            }
            Task::none()
        }
        Message::PathEdit(value) => {
            if let Some(tab) = app.active_mut() {
                tab.path_edit = value;
            }
            Task::none()
        }
        Message::GoUp => {
            if let Some(tab) = app.active() {
                let path = model::parent_remote(&tab.cwd);
                if path != tab.cwd {
                    let tab_id = tab.id;
                    app.send(WorkerIn::List { tab: tab_id, path });
                }
            }
            Task::none()
        }
        Message::GoHome => {
            if let Some(tab) = app.active() {
                let path = if tab.home.is_empty() {
                    "/".to_string()
                } else {
                    tab.home.clone()
                };
                let tab_id = tab.id;
                app.send(WorkerIn::List { tab: tab_id, path });
            }
            Task::none()
        }
        Message::SubmitPath | Message::RefreshDir => {
            if let Some(tab) = app.active() {
                let tab_id = tab.id;
                let path = if matches!(message, Message::RefreshDir) {
                    tab.cwd.clone()
                } else {
                    tab.path_edit.clone()
                };
                app.send(WorkerIn::List { tab: tab_id, path });
            }
            Task::none()
        }
        Message::SelectEntry(name) => {
            if let Some(tab) = app.active_mut() {
                tab.selected_entry = Some(name);
            }
            Task::none()
        }
        Message::OpenEntry(name) => {
            let action = app.active().map(|tab| {
                let entry = tab.entries.iter().find(|entry| entry.name == name);
                let is_dir = name == ".." || entry.map(|entry| entry.is_dir).unwrap_or(false);
                (tab.id, tab.cwd.clone(), name, is_dir)
            });
            if let Some((tab_id, cwd, name, is_dir)) = action {
                if is_dir {
                    app.send(WorkerIn::List {
                        tab: tab_id,
                        path: model::join_remote(&cwd, &name),
                    });
                } else {
                    return update(app, Message::Download);
                }
            }
            Task::none()
        }
        Message::Download => {
            let picked = app.active().and_then(|tab| {
                let name = tab.selected_entry.clone()?;
                let entry = tab.entries.iter().find(|entry| entry.name == name)?;
                if entry.is_dir {
                    return None;
                }
                Some((tab.id, model::join_remote(&tab.cwd, &name)))
            });
            let Some((tab_id, remote)) = picked else {
                app.status = "请选择一个文件再下载".to_string();
                return Task::none();
            };
            Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("选择保存位置")
                        .pick_folder()
                        .await
                        .map(|folder| folder.path().to_path_buf())
                },
                move |folder| Message::DownloadChosen {
                    tab: tab_id,
                    remote,
                    folder,
                },
            )
        }
        Message::DownloadChosen { tab, remote, folder } => {
            if let Some(folder) = folder {
                let name = remote
                    .rsplit('/')
                    .next()
                    .unwrap_or("download")
                    .to_string();
                app.send(WorkerIn::Download {
                    tab,
                    remote,
                    local: folder.join(name),
                });
                app.status = "正在下载…".to_string();
            }
            Task::none()
        }
        Message::Upload => {
            let Some(tab) = app.active() else {
                return Task::none();
            };
            let tab_id = tab.id;
            let remote_dir = tab.cwd.clone();
            Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("选择要上传的文件")
                        .pick_file()
                        .await
                        .map(|file| file.path().to_path_buf())
                },
                move |local| Message::UploadChosen {
                    tab: tab_id,
                    remote_dir,
                    local,
                },
            )
        }
        Message::UploadChosen {
            tab,
            remote_dir,
            local,
        } => {
            if let Some(local) = local {
                let name = local
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("upload");
                let remote = model::join_remote(&remote_dir, name);
                app.send(WorkerIn::Upload {
                    tab,
                    local,
                    remote,
                });
                app.status = "正在上传…".to_string();
            }
            Task::none()
        }
        Message::MkdirName(value) => {
            if let Some(tab) = app.active_mut() {
                tab.mkdir_name = value;
            }
            Task::none()
        }
        Message::CreateDir => {
            if let Some(tab) = app.active() {
                let name = tab.mkdir_name.trim().to_string();
                if name.is_empty() || name.contains('/') {
                    app.status = "请输入不含斜杠的目录名".to_string();
                } else {
                    let path = model::join_remote(&tab.cwd, &name);
                    let tab_id = tab.id;
                    app.send(WorkerIn::Mkdir { tab: tab_id, path });
                }
            }
            Task::none()
        }
        Message::AskDeleteRemote => {
            if let Some(tab) = app.active() {
                if let Some(name) = tab.selected_entry.clone() {
                    if name == ".." {
                        return Task::none();
                    }
                    let dir = tab
                        .entries
                        .iter()
                        .find(|entry| entry.name == name)
                        .map(|entry| entry.is_dir)
                        .unwrap_or(false);
                    app.dialog = Some(Dialog::DeleteRemote {
                        path: model::join_remote(&tab.cwd, &name),
                        dir,
                    });
                }
            }
            Task::none()
        }
        Message::DeleteRemote => {
            if let Some(Dialog::DeleteRemote { path, dir }) = app.dialog.take() {
                if let Some(tab) = app.active() {
                    app.send(WorkerIn::Remove {
                        tab: tab.id,
                        path,
                        dir,
                    });
                }
            }
            Task::none()
        }
    }
}
