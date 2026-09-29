use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::Cache;
use iced::widget::svg::{self, Svg};
use iced::widget::{
    button, canvas, checkbox, column, container, mouse_area, pick_list, progress_bar, row,
    scrollable, space, stack, text, text_input,
};
use iced::{
    Background, Border, Color, Element, Event, Fill, Font, Pixels, Point, Rectangle, Size,
    Subscription, Task, Theme,
};

use crate::model::{self, Auth, Profile};
use crate::ssh::{self, LinkState, WorkerIn, WorkerOut};
use crate::term::{self, InputEffect, KeyCode, KeyInput, Rgb, Terminal, DEFAULT_BG};

const FONT_SIZE: f32 = 15.0;
const CELL_H: f32 = 20.0;

const APP_BG: Color = Color::from_rgb8(11, 13, 16);
const SIDEBAR: Color = Color::from_rgb8(24, 27, 34);
const RAIL: Color = Color::from_rgb8(17, 19, 24);
const CARD: Color = Color::from_rgb8(34, 39, 50);
const LINE: Color = Color::from_rgb8(46, 53, 66);
const TEXT: Color = Color::from_rgb8(232, 236, 242);
const MUTED: Color = Color::from_rgb8(138, 148, 164);
const ACCENT: Color = Color::from_rgb8(56, 148, 236);
const ACCENT_HOVER: Color = Color::from_rgb8(70, 158, 236);
const ACCENT_PRESS: Color = Color::from_rgb8(30, 110, 190);
const OK: Color = Color::from_rgb8(62, 186, 140);
const DANGER: Color = Color::from_rgb8(224, 96, 102);
const WARN_BG: Color = Color::from_rgb8(58, 36, 28);

const AUTH_KINDS: [AuthKind; 2] = [AuthKind::Password, AuthKind::PrivateKey];

pub fn run() -> iced::Result {
    iced::application(boot, update, view)
        .title(title)
        .subscription(subscription)
        .theme(theme)
        .window_size(Size::new(1280.0, 800.0))
        .run()
}

fn boot() -> (App, Task<Message>) {
    (App::load(), Task::none())
}

fn title(app: &App) -> String {
    if let Some(tab) = app.active() {
        format!("{} — RustSSH", tab.display_title())
    } else {
        "RustSSH".to_string()
    }
}

fn theme(_: &App) -> Theme {
    Theme::Dark
}

fn subscription(app: &App) -> Subscription<Message> {
    let bridge = Subscription::run(ssh::bridge).map(Message::Worker);
    if app.sftp_drag.is_none() && app.sidebar_drag.is_none() && app.session_drag.is_none() {
        return bridge;
    }
    Subscription::batch([bridge, iced::event::listen_with(sftp_drag_events)])
}

fn sftp_drag_events(
    event: Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
            Some(Message::SftpDragMove(position.x, position.y))
        }
        Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            Some(Message::SftpDragEnd)
        }
        _ => None,
    }
}

struct App {
    profiles: Vec<Profile>,
    known: BTreeMap<String, String>,
    search: String,
    selected: Option<String>,
    collapsed: HashSet<String>,
    quick_target: String,
    quick_password: String,
    tabs: Vec<Tab>,
    active: Option<usize>,
    next_tab: u64,
    bridge: Option<ssh::BridgeTx>,
    queued: Vec<WorkerIn>,
    dialog: Option<Dialog>,
    show_sftp: bool,
    sftp_width: f32,
    sftp_drag: Option<SftpDrag>,
    sidebar_width: f32,
    sidebar_open: bool,
    sidebar_drag: Option<SidebarDrag>,
    session_drag: Option<SessionDrag>,
    sftp_scroll: SftpScroll,
    pane_cols: usize,
    pane_rows: usize,
    status: String,
}

#[derive(Clone)]
struct SessionDrag {
    id: String,
    origin: Option<Point>,
    dragging: bool,
    hover: Option<String>,
}

#[derive(Clone, Copy)]
struct SidebarDrag {
    anchor_x: Option<f32>,
    anchor_width: f32,
}

#[derive(Clone, Copy)]
struct SftpDrag {
    anchor_x: Option<f32>,
    anchor_y: Option<f32>,
    last_y: f32,
    anchor_width: f32,
    mode: RailDrag,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RailDrag {
    Pending,
    Resize,
    Scroll,
}

#[derive(Clone, Copy, Default)]
struct SftpScroll {
    offset_y: f32,
    view_h: f32,
    content_h: f32,
}

struct Tab {
    id: u64,
    gen: u64,
    #[allow(dead_code)]
    profile_id: Option<String>,
    name: String,
    remote_title: Option<String>,
    request: ssh::ConnectRequest,
    link: LinkState,
    fingerprint: String,
    term: Terminal,
    cache: Cache,
    cwd: String,
    home: String,
    path_edit: String,
    entries: Vec<ssh::RemoteEntry>,
    selected_entry: Option<String>,
    sftp_status: String,
    transfer: Option<ssh::Transfer>,
    stats: Option<ssh::RemoteStats>,
    selecting: bool,
    dragged: bool,
    mkdir_name: String,
}

impl Tab {
    fn display_title(&self) -> String {
        self.remote_title
            .clone()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Clone)]
enum Dialog {
    Editor(Editor),
    Password {
        profile_id: String,
        password: String,
        remember: bool,
    },
    DeleteSession {
        id: String,
        name: String,
    },
    DeleteRemote {
        path: String,
        dir: bool,
    },
}

#[derive(Clone)]
struct Editor {
    id: Option<String>,
    name: String,
    group: String,
    host: String,
    port: String,
    username: String,
    auth: AuthKind,
    password: String,
    remember: bool,
    key_path: String,
    passphrase: String,
    error: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthKind {
    Password,
    PrivateKey,
}

impl std::fmt::Display for AuthKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Password => "密码",
            Self::PrivateKey => "私钥",
        })
    }
}

#[derive(Clone, Debug)]
enum Pointer {
    Down(usize, usize),
    Drag(usize, usize),
    Up,
    Right,
}

#[derive(Clone)]
enum Message {
    Worker(WorkerOut),
    QuickTarget(String),
    QuickPassword(String),
    QuickConnect,
    Search(String),
    ConnectProfile(String),
    ConnectSelected,
    NewSession,
    EditSession,
    AskDeleteSession,
    DeleteSession,
    ClearHostKey,
    ToggleGroup(String),
    SessionGrab(String),
    SessionHoverEnter(String),
    SessionHoverExit(String),
    EditorName(String),
    EditorGroup(String),
    EditorHost(String),
    EditorPort(String),
    EditorUser(String),
    EditorAuth(AuthKind),
    EditorPassword(String),
    EditorRemember(bool),
    EditorKey(String),
    EditorPassphrase(String),
    BrowseKey,
    KeyPicked(Option<String>),
    SaveEditor,
    CloseDialog,
    PasswordChanged(String),
    PasswordRemember(bool),
    SubmitPassword,
    SelectTab(u64),
    CloseTab(u64),
    CycleTab(i32),
    Reconnect,
    ToggleSftp,
    SelectPane(bool),
    SidebarDragStart,
    SftpDragStart,
    SftpDragMove(f32, f32),
    SftpDragEnd,
    SftpScrolled(scrollable::Viewport),
    SftpRailWheel(f32),
    PaneResized(usize, usize),
    TermKey { tab: u64, input: KeyInput },
    Pointer { tab: u64, action: Pointer },
    Wheel { tab: u64, delta: f32 },
    Pasted(Option<String>),
    PathEdit(String),
    SubmitPath,
    RefreshDir,
    GoUp,
    GoHome,
    SelectEntry(String),
    OpenEntry(String),
    Download,
    DownloadChosen {
        tab: u64,
        remote: String,
        folder: Option<PathBuf>,
    },
    Upload,
    UploadChosen {
        tab: u64,
        remote_dir: String,
        local: Option<PathBuf>,
    },
    MkdirName(String),
    CreateDir,
    AskDeleteRemote,
    DeleteRemote,
    Nop,
}

impl App {
    fn load() -> Self {
        let (profiles, status) = match model::load_profiles() {
            Ok(profiles) => (profiles, "就绪".to_string()),
            Err(error) => (Vec::new(), error),
        };
        Self {
            profiles,
            known: model::load_known_hosts(),
            search: String::new(),
            selected: None,
            collapsed: HashSet::new(),
            quick_target: String::new(),
            quick_password: String::new(),
            tabs: Vec::new(),
            active: None,
            next_tab: 1,
            bridge: None,
            queued: Vec::new(),
            dialog: None,
            show_sftp: false,
            sftp_width: 320.0,
            sftp_drag: None,
            sidebar_width: 280.0,
            sidebar_open: false,
            sidebar_drag: None,
            session_drag: None,
            sftp_scroll: SftpScroll::default(),
            pane_cols: 0,
            pane_rows: 0,
            status,
        }
    }

    fn active(&self) -> Option<&Tab> {
        self.active.and_then(|index| self.tabs.get(index))
    }

    fn active_mut(&mut self) -> Option<&mut Tab> {
        let index = self.active?;
        self.tabs.get_mut(index)
    }

    fn tab_index(&self, id: u64, gen: Option<u64>) -> Option<usize> {
        self.tabs.iter().position(|tab| {
            tab.id == id && gen.map(|gen| tab.gen == gen).unwrap_or(true)
        })
    }

    fn send(&mut self, message: WorkerIn) {
        if let Some(bridge) = &self.bridge {
            bridge.send(message);
        } else {
            self.queued.push(message);
        }
    }

    fn selected_profile(&self) -> Option<&Profile> {
        let id = self.selected.as_ref()?;
        self.profiles.iter().find(|profile| &profile.id == id)
    }

    fn connect_profile(&mut self, id: &str, override_password: Option<String>, remember: bool) {
        let Some(index) = self.profiles.iter().position(|profile| profile.id == id) else {
            self.status = "会话不存在".to_string();
            return;
        };
        let mut profile = self.profiles[index].clone();
        let runtime = match &profile.auth {
            Auth::Password { saved } => {
                let password = if let Some(password) = override_password {
                    if remember {
                        profile.auth = Auth::Password {
                            saved: Some(password.clone()),
                        };
                        self.profiles[index] = profile.clone();
                        if let Err(error) = model::save_profiles(&self.profiles) {
                            self.status = error;
                        }
                    }
                    password
                } else if let Some(password) = saved.clone() {
                    password
                } else {
                    self.dialog = Some(Dialog::Password {
                        profile_id: id.to_string(),
                        password: String::new(),
                        remember: false,
                    });
                    return;
                };
                ssh::RuntimeAuth::Password(password)
            }
            Auth::PrivateKey { path, passphrase } => ssh::RuntimeAuth::PrivateKey {
                path: path.clone(),
                passphrase: passphrase.clone(),
            },
        };
        let known = self
            .known
            .get(&model::host_key(&profile.host, profile.port))
            .cloned();
        self.open_tab(
            profile.name.clone(),
            Some(profile.id.clone()),
            profile.host.clone(),
            profile.port,
            profile.username.clone(),
            runtime,
            known,
        );
        self.selected = Some(id.to_string());
    }

    fn open_tab(
        &mut self,
        name: String,
        profile_id: Option<String>,
        host: String,
        port: u16,
        username: String,
        auth: ssh::RuntimeAuth,
        known: Option<String>,
    ) {
        let id = self.next_tab;
        self.next_tab += 1;
        let cols = if self.pane_cols >= 2 { self.pane_cols } else { 80 };
        let rows = if self.pane_rows >= 2 { self.pane_rows } else { 24 };
        let request = ssh::ConnectRequest {
            tab: id,
            gen: 1,
            host: host.clone(),
            port,
            username: username.clone(),
            auth,
            cols: cols as u32,
            rows: rows as u32,
            known_fingerprint: known,
        };
        self.tabs.push(Tab {
            id,
            gen: 1,
            profile_id,
            name,
            remote_title: None,
            request: request.clone(),
            link: LinkState::Connecting,
            fingerprint: String::new(),
            term: Terminal::new(cols, rows),
            cache: Cache::new(),
            cwd: "/".to_string(),
            home: String::new(),
            path_edit: "/".to_string(),
            entries: Vec::new(),
            selected_entry: None,
            sftp_status: String::new(),
            transfer: None,
            stats: None,
            selecting: false,
            dragged: false,
            mkdir_name: String::new(),
        });
        self.active = Some(self.tabs.len() - 1);
        self.send(WorkerIn::Open(request));
        self.status = format!("正在连接 {username}@{host}:{port}");
    }

    fn close_tab(&mut self, id: u64) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.send(WorkerIn::Close { tab: id });
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active = None;
        } else if let Some(active) = self.active {
            if index < active {
                self.active = Some(active - 1);
            } else if index == active {
                self.active = Some(index.min(self.tabs.len() - 1));
            }
        }
        self.sync_pty();
    }

    fn sync_pty(&mut self) {
        let cols = self.pane_cols;
        let rows = self.pane_rows;
        if cols < 2 || rows < 2 {
            return;
        }
        let Some(index) = self.active else {
            return;
        };
        let resized = {
            let Some(tab) = self.tabs.get_mut(index) else {
                return;
            };
            if !tab.term.resize(cols, rows) {
                return;
            }
            tab.request.cols = cols as u32;
            tab.request.rows = rows as u32;
            tab.cache.clear();
            tab.id
        };
        self.send(WorkerIn::Resize {
            tab: resized,
            cols: cols as u32,
            rows: rows as u32,
        });
    }

    fn reconnect(&mut self) {
        let cols = if self.pane_cols >= 2 { self.pane_cols } else { 80 };
        let rows = if self.pane_rows >= 2 { self.pane_rows } else { 24 };
        let Some(index) = self.active else {
            return;
        };
        let host_key = model::host_key(&self.tabs[index].request.host, self.tabs[index].request.port);
        let known = self.known.get(&host_key).cloned();
        let tab = &mut self.tabs[index];
        tab.gen += 1;
        tab.request.gen = tab.gen;
        tab.request.cols = cols as u32;
        tab.request.rows = rows as u32;
        tab.request.known_fingerprint = known;
        tab.link = LinkState::Connecting;
        tab.remote_title = None;
        tab.term = Terminal::new(cols, rows);
        tab.cache.clear();
        tab.entries.clear();
        tab.cwd = "/".to_string();
        tab.home.clear();
        tab.path_edit = "/".to_string();
        tab.sftp_status.clear();
        tab.transfer = None;
        tab.stats = None;
        tab.fingerprint.clear();
        let request = tab.request.clone();
        let label = format!("{}@{}:{}", request.username, request.host, request.port);
        self.send(WorkerIn::Open(request));
        self.status = format!("正在重新连接 {label}");
    }

    fn send_input(&mut self, tab: u64, data: Vec<u8>) {
        let online = self
            .tabs
            .iter()
            .any(|item| item.id == tab && matches!(item.link, LinkState::Online));
        if online && !data.is_empty() {
            self.send(WorkerIn::Input { tab, data });
        }
    }

    fn clear_host_key(&mut self) {
        let target = if let Some(tab) = self.active() {
            Some((tab.request.host.clone(), tab.request.port))
        } else {
            self.selected_profile()
                .map(|profile| (profile.host.clone(), profile.port))
        };
        let Some((host, port)) = target else {
            self.status = "没有可清除的主机".to_string();
            return;
        };
        self.known.remove(&model::host_key(&host, port));
        if let Err(error) = model::save_known_hosts(&self.known) {
            self.status = error;
            return;
        }
        if let Some(tab) = self.active_mut() {
            if tab.request.host == host && tab.request.port == port {
                tab.request.known_fingerprint = None;
            }
        }
        self.status = format!("已清除 {host}:{port} 的主机密钥");
    }

    fn save_editor(&mut self) {
        let Some(Dialog::Editor(editor)) = self.dialog.clone() else {
            return;
        };
        let port = match editor.port.trim().parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => {
                self.set_editor_error("端口必须是 1 到 65535 的数字");
                return;
            }
        };
        if editor.host.trim().is_empty() || editor.username.trim().is_empty() {
            self.set_editor_error("主机和用户名不能为空");
            return;
        }
        if matches!(editor.auth, AuthKind::PrivateKey) && editor.key_path.trim().is_empty() {
            self.set_editor_error("请选择私钥文件");
            return;
        }
        let name = if editor.name.trim().is_empty() {
            format!("{}@{}", editor.username.trim(), editor.host.trim())
        } else {
            editor.name.trim().to_string()
        };
        let auth = match editor.auth {
            AuthKind::Password => Auth::Password {
                saved: editor.remember.then(|| editor.password.clone()),
            },
            AuthKind::PrivateKey => Auth::PrivateKey {
                path: editor.key_path.trim().to_string(),
                passphrase: (!editor.passphrase.is_empty()).then(|| editor.passphrase.clone()),
            },
        };
        let profile = Profile {
            id: editor
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            group: editor.group.trim().to_string(),
            host: editor.host.trim().to_string(),
            port,
            username: editor.username.trim().to_string(),
            auth,
        };
        if let Some(index) = self.profiles.iter().position(|item| item.id == profile.id) {
            self.profiles[index] = profile.clone();
        } else {
            self.profiles.push(profile.clone());
        }
        if let Err(error) = model::save_profiles(&self.profiles) {
            self.status = error;
            return;
        }
        self.selected = Some(profile.id);
        self.dialog = None;
        self.status = "会话已保存".to_string();
    }

    fn set_editor_error(&mut self, error: &str) {
        if let Some(Dialog::Editor(editor)) = &mut self.dialog {
            editor.error = error.to_string();
        }
    }

    fn editor_from(profile: &Profile) -> Editor {
        let (auth, password, remember, key_path, passphrase) = match &profile.auth {
            Auth::Password { saved } => (
                AuthKind::Password,
                saved.clone().unwrap_or_default(),
                saved.is_some(),
                String::new(),
                String::new(),
            ),
            Auth::PrivateKey { path, passphrase } => (
                AuthKind::PrivateKey,
                String::new(),
                false,
                path.clone(),
                passphrase.clone().unwrap_or_default(),
            ),
        };
        Editor {
            id: Some(profile.id.clone()),
            name: profile.name.clone(),
            group: profile.group.clone(),
            host: profile.host.clone(),
            port: profile.port.to_string(),
            username: profile.username.clone(),
            auth,
            password,
            remember,
            key_path,
            passphrase,
            error: String::new(),
        }
    }
}

fn update(app: &mut App, message: Message) -> Task<Message> {
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

fn view(app: &App) -> Element<'_, Message> {
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

fn top_bar(app: &App) -> Element<'_, Message> {
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

fn hairline() -> Element<'static, Message> {
    container(column![])
        .width(Fill)
        .height(1)
        .style(|_| container::Style {
            background: Some(Background::Color(LINE)),
            ..Default::default()
        })
        .into()
}

fn body(app: &App) -> Element<'_, Message> {
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

fn pane_tabs(files: bool) -> Element<'static, Message> {
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

fn ssh_icon() -> svg::Handle {
    static ICON: OnceLock<svg::Handle> = OnceLock::new();
    ICON.get_or_init(|| svg::Handle::from_memory(include_bytes!("../assets/ssh.svg")))
        .clone()
}

fn files_icon() -> svg::Handle {
    static ICON: OnceLock<svg::Handle> = OnceLock::new();
    ICON.get_or_init(|| svg::Handle::from_memory(include_bytes!("../assets/files.svg")))
        .clone()
}

fn pane_tab(icon: svg::Handle, active: bool, message: Message) -> Element<'static, Message> {
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

fn pane_tab_style(active: bool, status: button::Status) -> button::Style {
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

fn sidebar(app: &App) -> Element<'_, Message> {
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

fn sidebar_splitter() -> Element<'static, Message> {
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

fn sidebar_drag_move(app: &mut App, x: f32) -> Task<Message> {
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

fn session_label(profile: &Profile) -> String {
    let target = format!("{}@{}:{}", profile.username, profile.host, profile.port);
    let name = profile.name.trim();
    if name.is_empty() || name == profile.host || name == format!("{}:{}", profile.host, profile.port) {
        target
    } else {
        format!("{name}  {target}")
    }
}

fn session_drag_move(app: &mut App, x: f32, y: f32) -> Task<Message> {
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

fn finish_session_drag(app: &mut App) {
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

fn session_row(profile: &Profile, selected: bool, dragging: bool, held: bool) -> Element<'_, Message> {
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

fn workspace(app: &App) -> Element<'_, Message> {
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

fn tab_strip(app: &App) -> Element<'_, Message> {
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

fn link_banner(tab: &Tab) -> Element<'_, Message> {
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

fn banner<'a>(message: String, actions: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
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

fn terminal_row<'a>(app: &'a App, tab: &'a Tab) -> Element<'a, Message> {
    container(term_canvas(tab, app.dialog.is_none()))
        .width(Fill)
        .height(Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(rgb(DEFAULT_BG))),
            ..Default::default()
        })
        .into()
}

fn empty_state() -> Element<'static, Message> {
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

fn status_bar<'a>(app: &'a App, tab: Option<&'a Tab>) -> Element<'a, Message> {
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

fn monitor_bar(tab: Option<&Tab>) -> Element<'_, Message> {
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

fn cpu_chip(percent: f32) -> Element<'static, Message> {
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

fn stat_chip(label: String) -> Element<'static, Message> {
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

fn pick_disks(disks: &[ssh::DiskUsage]) -> Vec<&ssh::DiskUsage> {
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

fn format_uptime(secs: u64) -> String {
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

fn sftp_panel(app: &App) -> Element<'_, Message> {
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

fn sftp_body<'a>(tab: &'a Tab, scroll: &'a SftpScroll) -> Element<'a, Message> {
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

fn sftp_files(files: iced::widget::Column<'_, Message>) -> Element<'_, Message> {
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

fn sftp_rail(scroll: &SftpScroll) -> Element<'_, Message> {
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

fn rail_thumb(scroll: &SftpScroll) -> Element<'static, Message> {
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

fn portion(value: u16) -> iced::Length {
    if value == 0 {
        iced::Length::Fixed(0.0)
    } else {
        iced::Length::FillPortion(value)
    }
}

fn sftp_drag_move(app: &mut App, x: f32, y: f32) -> Task<Message> {
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

fn scroll_scale(scroll: &SftpScroll) -> f32 {
    let thumb = scroll.view_h * (scroll.view_h / scroll.content_h.max(1.0));
    let travel = (scroll.view_h - thumb).max(1.0);
    let range = (scroll.content_h - scroll.view_h).max(0.0);
    if scroll.view_h <= 1.0 {
        1.0
    } else {
        range / travel
    }
}

fn scroll_files(delta_y: f32) -> Task<Message> {
    if delta_y == 0.0 {
        return Task::none();
    }
    iced::widget::operation::scroll_by(
        iced::widget::Id::new("sftp-files"),
        scrollable::AbsoluteOffset { x: 0.0, y: delta_y },
    )
}

fn entry_row<'a>(name: &'a str, meta: impl Into<String>, selected: bool) -> Element<'a, Message> {
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

fn dialog_card(dialog: &Dialog) -> Element<'_, Message> {
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

fn editor_card(editor: &Editor) -> Element<'_, Message> {
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

fn card<'a>(title: &'a str, body: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
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

fn labeled<'a>(label: &'a str, field: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![text(label).size(12).color(MUTED), field.into()]
        .spacing(4)
        .width(Fill)
        .into()
}

fn accent_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(13))
        .on_press(message)
        .padding([7, 12])
        .style(primary_style)
        .into()
}

fn tool_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(12).color(TEXT))
        .on_press(message)
        .padding([4, 7])
        .style(ghost_style)
        .into()
}

fn ghost_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(13).color(TEXT))
        .on_press(message)
        .padding([7, 12])
        .style(ghost_style)
        .into()
}

fn danger_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(13))
        .on_press(message)
        .padding([7, 12])
        .style(danger_style)
        .into()
}

fn panel(color: Color) -> container::Style {
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

fn field_style(_theme: &Theme, status: text_input::Status) -> text_input::Style {
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

fn primary_style(_theme: &Theme, status: button::Status) -> button::Style {
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

fn ghost_style(_theme: &Theme, status: button::Status) -> button::Style {
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

fn danger_style(_theme: &Theme, status: button::Status) -> button::Style {
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

fn tab_chip(active: bool) -> container::Style {
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

fn close_style(_theme: &Theme, status: button::Status) -> button::Style {
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

fn term_canvas(tab: &Tab, accept_keys: bool) -> Element<'_, Message> {
    canvas(TermView {
        tab_id: tab.id,
        term: &tab.term,
        cache: &tab.cache,
        accept_keys,
        selecting: tab.selecting,
    })
    .width(Fill)
    .height(Fill)
    .into()
}

struct TermView<'a> {
    tab_id: u64,
    term: &'a Terminal,
    cache: &'a Cache,
    accept_keys: bool,
    selecting: bool,
}

impl canvas::Program<Message> for TermView<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            paint_term(frame, self.term, bounds.size());
        });
        vec![geometry]
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let Some((cols, rows)) = grid_for(bounds) {
            if cols != self.term.cols() || rows != self.term.rows() {
                return Some(canvas::Action::publish(Message::PaneResized(cols, rows)));
            }
        }
        match event {
            Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key,
                modifiers,
                text,
                ..
            }) if self.accept_keys => {
                if modifiers.control()
                    && !modifiers.alt()
                    && matches!(key, iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab))
                {
                    let delta = if modifiers.shift() { -1 } else { 1 };
                    return Some(canvas::Action::publish(Message::CycleTab(delta)));
                }
                let input = KeyInput {
                    key: map_key(key, text.as_deref())?,
                    text: text.as_ref().map(|value| value.to_string()),
                    ctrl: modifiers.control(),
                    alt: modifiers.alt(),
                    shift: modifiers.shift(),
                    logo: modifiers.logo(),
                };
                Some(canvas::Action::publish(Message::TermKey {
                    tab: self.tab_id,
                    input,
                }))
            }
            Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)) => {
                let (col, row) = hit(self.term, bounds, cursor)?;
                Some(canvas::Action::publish(Message::Pointer {
                    tab: self.tab_id,
                    action: Pointer::Down(col, row),
                }))
            }
            Event::Mouse(iced::mouse::Event::CursorMoved { .. }) if self.selecting => {
                let (col, row) = hit(self.term, bounds, cursor)?;
                Some(canvas::Action::publish(Message::Pointer {
                    tab: self.tab_id,
                    action: Pointer::Drag(col, row),
                }))
            }
            Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))
                if self.selecting =>
            {
                Some(canvas::Action::publish(Message::Pointer {
                    tab: self.tab_id,
                    action: Pointer::Up,
                }))
            }
            Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Right)) => {
                Some(canvas::Action::publish(Message::Pointer {
                    tab: self.tab_id,
                    action: Pointer::Right,
                }))
            }
            Event::Mouse(iced::mouse::Event::WheelScrolled { delta }) => {
                let y = match delta {
                    iced::mouse::ScrollDelta::Lines { y, .. } => *y,
                    iced::mouse::ScrollDelta::Pixels { y, .. } => *y / CELL_H,
                };
                if y.abs() < f32::EPSILON {
                    None
                } else {
                    Some(canvas::Action::publish(Message::Wheel {
                        tab: self.tab_id,
                        delta: y,
                    }))
                }
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> iced::mouse::Interaction {
        iced::mouse::Interaction::Text
    }
}

fn paint_term(frame: &mut canvas::Frame, term: &Terminal, size: Size) {
    frame.fill_rectangle(Point::new(0.0, 0.0), size, rgb(DEFAULT_BG));
    let cursor = term.cursor_view_pos();
    let mut runs = Vec::new();
    for row in 0..term.rows() {
        let mut col = 0;
        while col < term.cols() {
            let cell = term.cell_at_view(row, col);
            if cell.width == 0 {
                col += 1;
                continue;
            }
            let advance = usize::from(cell.width.max(1));
            let style = cell_style(&cell, term.is_selected(row, col));
            if cell.ch.is_ascii() {
                let start = col;
                let mut content = String::new();
                content.push(cell.ch);
                col += advance;
                while col < term.cols() {
                    let next = term.cell_at_view(row, col);
                    if next.width == 0 || !next.ch.is_ascii() {
                        break;
                    }
                    let next_advance = usize::from(next.width.max(1));
                    if cell_style(&next, term.is_selected(row, col)) != style {
                        break;
                    }
                    content.push(next.ch);
                    col += next_advance;
                }
                runs.push((content, row, start, col, style, false));
            } else {
                let start = col;
                col += advance;
                runs.push((cell.ch.to_string(), row, start, col, style, true));
            }
        }
    }
    for (_, row, start, end, style, _) in &runs {
        paint_run_bg(frame, *row, *start, *end, *style);
    }
    for (content, row, start, end, style, fallback) in &runs {
        paint_run_text(frame, content, *row, *start, *end, *style, *fallback);
    }
    if let Some((col, row)) = cursor {
        let x = col as f32 * cell_w();
        let y = row as f32 * CELL_H;
        frame.fill_rectangle(
            Point::new(x, y + 3.0),
            Size::new(2.0, CELL_H - 6.0),
            Color::from_rgb8(236, 239, 244),
        );
    }
}

fn paint_run_bg(
    frame: &mut canvas::Frame,
    row: usize,
    start: usize,
    end: usize,
    style: (Rgb, Rgb, bool, bool, bool),
) {
    let (_, bg, _, _, _) = style;
    let x = start as f32 * cell_w();
    let y = row as f32 * CELL_H;
    let width = (end - start) as f32 * cell_w();
    frame.fill_rectangle(Point::new(x, y), Size::new(width, CELL_H), rgb(bg));
}

fn paint_run_text(
    frame: &mut canvas::Frame,
    content: &str,
    row: usize,
    start: usize,
    end: usize,
    style: (Rgb, Rgb, bool, bool, bool),
    fallback: bool,
) {
    let (fg, _, bold, italic, underline) = style;
    let x = start as f32 * cell_w();
    let y = row as f32 * CELL_H;
    let width = (end - start) as f32 * cell_w();
    if content.chars().any(|ch| ch != ' ') {
        let size = if fallback { wide_size() } else { FONT_SIZE };
        let mut drawn = canvas::Text::from(content);
        drawn.position = Point::new(x, y + ((CELL_H - size) / 2.0).max(0.0));
        drawn.color = rgb(fg);
        drawn.size = Pixels(size);
        drawn.font = mono(bold, italic);
        drawn.shaping = if fallback {
            iced::widget::text::Shaping::Advanced
        } else {
            iced::widget::text::Shaping::Basic
        };
        drawn.align_x = Horizontal::Left.into();
        drawn.align_y = Vertical::Top;
        frame.fill_text(drawn);
    }
    if underline {
        frame.fill_rectangle(
            Point::new(x, y + CELL_H - 2.0),
            Size::new(width, 1.0),
            rgb(fg),
        );
    }
}

fn wide_size() -> f32 {
    static SIZE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        let sample = "中";
        let width = measure_text_advanced(sample);
        let target = cell_w() * 2.0;
        if width.is_finite() && width > 1.0 {
            (FONT_SIZE * target / width).clamp(8.0, CELL_H - 2.0)
        } else {
            FONT_SIZE
        }
    })
}

fn cell_style(cell: &term::Cell, selected: bool) -> (Rgb, Rgb, bool, bool, bool) {
    let (fg, mut bg) = if cell.inverse {
        (cell.bg, cell.fg)
    } else {
        (cell.fg, cell.bg)
    };
    if selected {
        bg = Rgb::new(38, 79, 120);
    }
    (fg, bg, cell.bold, cell.italic, cell.underline)
}

fn cell_w() -> f32 {
    static WIDTH: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *WIDTH.get_or_init(|| {
        let sample = "00000000000000000000000000000000";
        let width = measure_text(sample) / sample.len() as f32;
        if width.is_finite() && (4.0..=24.0).contains(&width) {
            width
        } else {
            9.0
        }
    })
}

fn measure_text(content: &str) -> f32 {
    measure_text_with(content, iced::widget::text::Shaping::Basic)
}

fn measure_text_advanced(content: &str) -> f32 {
    measure_text_with(content, iced::widget::text::Shaping::Advanced)
}

fn measure_text_with(content: &str, shaping: iced::widget::text::Shaping) -> f32 {
    use iced_graphics::core::text::Paragraph as _;

    let paragraph =
        iced_graphics::text::Paragraph::with_text(iced_graphics::core::text::Text {
            content,
            bounds: Size::new(f32::INFINITY, f32::INFINITY),
            size: Pixels(FONT_SIZE),
            line_height: iced::widget::text::LineHeight::default(),
            font: mono(false, false),
            align_x: Horizontal::Left.into(),
            align_y: Vertical::Top,
            shaping,
            wrapping: iced::widget::text::Wrapping::None,
        });
    paragraph.min_width()
}

fn mono(bold: bool, italic: bool) -> Font {
    Font {
        weight: if bold {
            iced::font::Weight::Bold
        } else {
            iced::font::Weight::Normal
        },
        style: if italic {
            iced::font::Style::Italic
        } else {
            iced::font::Style::Normal
        },
        ..Font::MONOSPACE
    }
}

fn grid_for(bounds: Rectangle) -> Option<(usize, usize)> {
    if bounds.width < cell_w() * 2.0 || bounds.height < CELL_H * 2.0 {
        return None;
    }
    Some((
        ((bounds.width / cell_w()).floor() as usize).max(2),
        ((bounds.height / CELL_H).floor() as usize).max(2),
    ))
}

fn hit(term: &Terminal, bounds: Rectangle, cursor: iced::mouse::Cursor) -> Option<(usize, usize)> {
    let point = cursor.position_in(bounds)?;
    let col = (point.x / cell_w()) as usize;
    let row = (point.y / CELL_H) as usize;
    if col < term.cols() && row < term.rows() {
        Some((col, row))
    } else {
        None
    }
}

fn map_key(key: &iced::keyboard::Key, text: Option<&str>) -> Option<KeyCode> {
    match key.as_ref() {
        iced::keyboard::Key::Named(named) => match named {
            iced::keyboard::key::Named::Enter => Some(KeyCode::Enter),
            iced::keyboard::key::Named::Backspace => Some(KeyCode::Backspace),
            iced::keyboard::key::Named::Tab => Some(KeyCode::Tab),
            iced::keyboard::key::Named::Escape => Some(KeyCode::Escape),
            iced::keyboard::key::Named::ArrowUp => Some(KeyCode::Up),
            iced::keyboard::key::Named::ArrowDown => Some(KeyCode::Down),
            iced::keyboard::key::Named::ArrowLeft => Some(KeyCode::Left),
            iced::keyboard::key::Named::ArrowRight => Some(KeyCode::Right),
            iced::keyboard::key::Named::Home => Some(KeyCode::Home),
            iced::keyboard::key::Named::End => Some(KeyCode::End),
            iced::keyboard::key::Named::PageUp => Some(KeyCode::PageUp),
            iced::keyboard::key::Named::PageDown => Some(KeyCode::PageDown),
            iced::keyboard::key::Named::Insert => Some(KeyCode::Insert),
            iced::keyboard::key::Named::Delete => Some(KeyCode::Delete),
            iced::keyboard::key::Named::Space => Some(KeyCode::Char(' ')),
            iced::keyboard::key::Named::F1 => Some(KeyCode::F(1)),
            iced::keyboard::key::Named::F2 => Some(KeyCode::F(2)),
            iced::keyboard::key::Named::F3 => Some(KeyCode::F(3)),
            iced::keyboard::key::Named::F4 => Some(KeyCode::F(4)),
            iced::keyboard::key::Named::F5 => Some(KeyCode::F(5)),
            iced::keyboard::key::Named::F6 => Some(KeyCode::F(6)),
            iced::keyboard::key::Named::F7 => Some(KeyCode::F(7)),
            iced::keyboard::key::Named::F8 => Some(KeyCode::F(8)),
            iced::keyboard::key::Named::F9 => Some(KeyCode::F(9)),
            iced::keyboard::key::Named::F10 => Some(KeyCode::F(10)),
            iced::keyboard::key::Named::F11 => Some(KeyCode::F(11)),
            iced::keyboard::key::Named::F12 => Some(KeyCode::F(12)),
            _ => None,
        },
        iced::keyboard::Key::Character(value) => value.chars().next().map(KeyCode::Char),
        iced::keyboard::Key::Unidentified => text.and_then(|value| value.chars().next()).map(KeyCode::Char),
    }
}

fn plain_key(key: KeyCode) -> KeyInput {
    KeyInput {
        key,
        text: None,
        ctrl: false,
        alt: false,
        shift: false,
        logo: false,
    }
}

fn grouped(app: &App) -> Vec<(String, Vec<&Profile>)> {
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

fn matches_search(profile: &Profile, query: &str) -> bool {
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

fn ellipsize(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn short_text(value: &str, max: usize) -> String {
    ellipsize(value, max)
}

fn transfer_view(transfer: Option<&ssh::Transfer>) -> Element<'_, Message> {
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

fn transfer_label(transfer: &ssh::Transfer) -> String {
    let verb = if transfer.upload { "上传" } else { "下载" };
    let name = ellipsize(&transfer.name, 18);
    let speed = format!("{}/s", human_size(transfer.speed));
    if transfer.total > 0 {
        let pct = (transfer.done as f64 / transfer.total as f64 * 100.0).min(100.0);
        format!(
            "{verb} {name}  {pct:.0}%  {} / {}  ·  {speed}",
            human_size(transfer.done),
            human_size(transfer.total),
        )
    } else {
        format!("{verb} {name}  {}  ·  {speed}", human_size(transfer.done))
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn rgb(color: Rgb) -> Color {
    Color::from_rgb8(color.r, color.g, color.b)
}
