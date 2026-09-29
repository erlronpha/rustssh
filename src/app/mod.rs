use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use iced::widget::canvas::Cache;
use iced::widget::scrollable;
use iced::{Event, Point, Size, Subscription, Task, Theme};

use crate::model::{self, Auth, Profile};
use crate::ssh::{self, LinkState, WorkerIn, WorkerOut};
use crate::term::{InputEffect, KeyCode, KeyInput, Terminal};


mod chrome;
mod update;
mod dialog;
mod format;
mod sftp_ui;
mod sidebar;
mod style;
mod term_view;
mod workspace;

pub(crate) use format::*;
pub(crate) use style::*;
use chrome::view;
use update::update;

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
pub(crate) enum AuthKind {
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
pub(crate) enum Pointer {
    Down(usize, usize),
    Drag(usize, usize),
    Up,
    Right,
}

#[derive(Clone)]
pub(crate) enum Message {
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

