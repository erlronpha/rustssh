use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::futures::SinkExt;
use iced::futures::Stream;
use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[derive(Clone)]
pub struct ConnectRequest {
    pub tab: u64,
    pub gen: u64,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: RuntimeAuth,
    pub cols: u32,
    pub rows: u32,
    pub known_fingerprint: Option<String>,
}

#[derive(Clone)]
pub enum RuntimeAuth {
    Password(String),
    PrivateKey {
        path: String,
        passphrase: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum LinkState {
    Connecting,
    Online,
    Offline(String),
}

#[derive(Clone, Debug)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub enum WorkerOut {
    Ready(BridgeTx),
    Data {
        tab: u64,
        gen: u64,
        bytes: Vec<u8>,
    },
    Status {
        tab: u64,
        gen: u64,
        state: LinkState,
    },
    HostKey {
        tab: u64,
        gen: u64,
        fingerprint: String,
    },
    Dir {
        tab: u64,
        gen: u64,
        path: String,
        entries: Vec<RemoteEntry>,
    },
    Notice {
        tab: u64,
        gen: u64,
        text: String,
        refresh: bool,
    },
    Transfer {
        tab: u64,
        gen: u64,
        transfer: Transfer,
    },
    Stats {
        tab: u64,
        gen: u64,
        stats: RemoteStats,
    },
}

#[derive(Clone, Debug)]
pub struct RemoteStats {
    pub host: String,
    pub cpu_percent: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub uptime_secs: u64,
    pub sessions: u32,
    pub disks: Vec<DiskUsage>,
}

#[derive(Clone, Debug)]
pub struct DiskUsage {
    pub mount: String,
    pub percent: u8,
}

#[derive(Clone, Debug)]
pub struct Transfer {
    pub upload: bool,
    pub name: String,
    pub done: u64,
    pub total: u64,
    pub speed: u64,
}

pub enum WorkerIn {
    Open(ConnectRequest),
    Input {
        tab: u64,
        data: Vec<u8>,
    },
    Resize {
        tab: u64,
        cols: u32,
        rows: u32,
    },
    Close {
        tab: u64,
    },
    List {
        tab: u64,
        path: String,
    },
    Download {
        tab: u64,
        remote: String,
        local: PathBuf,
    },
    Upload {
        tab: u64,
        local: PathBuf,
        remote: String,
    },
    Mkdir {
        tab: u64,
        path: String,
    },
    Remove {
        tab: u64,
        path: String,
        dir: bool,
    },
}

#[derive(Clone)]
pub struct BridgeTx(UnboundedSender<WorkerIn>);

impl BridgeTx {
    pub fn send(&self, message: WorkerIn) {
        let _ = self.0.send(message);
    }
}

impl std::fmt::Debug for BridgeTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BridgeTx")
    }
}

enum SessionCmd {
    Data(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    List(String),
    Download { remote: String, local: PathBuf },
    Upload { local: PathBuf, remote: String },
    Mkdir(String),
    Remove { path: String, dir: bool },
}

struct Live {
    tx: UnboundedSender<SessionCmd>,
    handle: tokio::task::JoinHandle<()>,
}

pub fn bridge() -> impl Stream<Item = WorkerOut> {
    iced::stream::channel(128, async |mut output| {
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (evt_tx, mut evt_rx) = tokio::sync::mpsc::unbounded_channel();
        if output
            .send(WorkerOut::Ready(BridgeTx(cmd_tx)))
            .await
            .is_err()
        {
            return;
        }

        let mut lives: HashMap<u64, Live> = HashMap::new();
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(message) => dispatch(message, &mut lives, &evt_tx),
                        None => break,
                    }
                }
                evt = evt_rx.recv() => {
                    match evt {
                        Some(event) => {
                            if output.send(event).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
        for (_, live) in lives {
            live.handle.abort();
        }
    })
}

fn dispatch(message: WorkerIn, lives: &mut HashMap<u64, Live>, evt_tx: &UnboundedSender<WorkerOut>) {
    match message {
        WorkerIn::Open(request) => {
            if let Some(old) = lives.remove(&request.tab) {
                old.handle.abort();
            }
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let events = evt_tx.clone();
            let tab = request.tab;
            let handle = tokio::spawn(async move {
                run_session(request, rx, events).await;
            });
            lives.insert(tab, Live { tx, handle });
        }
        WorkerIn::Input { tab, data } => {
            if let Some(live) = lives.get(&tab) {
                let _ = live.tx.send(SessionCmd::Data(data));
            }
        }
        WorkerIn::Resize { tab, cols, rows } => {
            if let Some(live) = lives.get(&tab) {
                let _ = live.tx.send(SessionCmd::Resize { cols, rows });
            }
        }
        WorkerIn::Close { tab } => {
            if let Some(live) = lives.remove(&tab) {
                live.handle.abort();
            }
        }
        WorkerIn::List { tab, path } => {
            if let Some(live) = lives.get(&tab) {
                let _ = live.tx.send(SessionCmd::List(path));
            }
        }
        WorkerIn::Download { tab, remote, local } => {
            if let Some(live) = lives.get(&tab) {
                let _ = live.tx.send(SessionCmd::Download { remote, local });
            }
        }
        WorkerIn::Upload { tab, local, remote } => {
            if let Some(live) = lives.get(&tab) {
                let _ = live.tx.send(SessionCmd::Upload { local, remote });
            }
        }
        WorkerIn::Mkdir { tab, path } => {
            if let Some(live) = lives.get(&tab) {
                let _ = live.tx.send(SessionCmd::Mkdir(path));
            }
        }
        WorkerIn::Remove { tab, path, dir } => {
            if let Some(live) = lives.get(&tab) {
                let _ = live.tx.send(SessionCmd::Remove { path, dir });
            }
        }
    }
}

struct ClientHandler {
    known: Option<String>,
    fingerprint: Arc<Mutex<Option<String>>>,
    mismatch: Arc<AtomicBool>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = fingerprint_of(server_public_key);
        if let Ok(mut slot) = self.fingerprint.lock() {
            *slot = Some(fingerprint.clone());
        }
        if let Some(known) = &self.known {
            if known != &fingerprint {
                self.mismatch.store(true, Ordering::SeqCst);
                return Ok(false);
            }
        }
        Ok(true)
    }
}

async fn run_session(
    request: ConnectRequest,
    mut cmd_rx: UnboundedReceiver<SessionCmd>,
    evt: UnboundedSender<WorkerOut>,
) {
    let tab = request.tab;
    let gen = request.gen;
    let fingerprint = Arc::new(Mutex::new(None));
    let mismatch = Arc::new(AtomicBool::new(false));
    let handler = ClientHandler {
        known: request.known_fingerprint.clone(),
        fingerprint: fingerprint.clone(),
        mismatch: mismatch.clone(),
    };
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(20)),
        keepalive_max: 3,
        nodelay: true,
        ..client::Config::default()
    });

    let connect = client::connect(
        config,
        (request.host.as_str(), request.port),
        handler,
    );
    let mut session = match tokio::time::timeout(Duration::from_secs(20), connect).await {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => {
            let reason = if mismatch.load(Ordering::SeqCst) {
                let seen = fingerprint
                    .lock()
                    .ok()
                    .and_then(|slot| slot.clone())
                    .unwrap_or_default();
                format!(
                    "主机密钥与已保存的记录不一致（{seen}）。确认服务器安全后，清除该主机密钥再连接。"
                )
            } else {
                format!("连接失败：{error}")
            };
            let _ = evt.send(WorkerOut::Status {
                tab,
                gen,
                state: LinkState::Offline(reason),
            });
            return;
        }
        Err(_) => {
            let _ = evt.send(WorkerOut::Status {
                tab,
                gen,
                state: LinkState::Offline("连接超时".to_string()),
            });
            return;
        }
    };

    if let Err(error) = authenticate(&mut session, &request).await {
        let _ = evt.send(WorkerOut::Status {
            tab,
            gen,
            state: LinkState::Offline(error),
        });
        return;
    }

    if let Some(fp) = fingerprint.lock().ok().and_then(|slot| slot.clone()) {
        let _ = evt.send(WorkerOut::HostKey {
            tab,
            gen,
            fingerprint: fp,
        });
    }

    let mut channel = match session.channel_open_session().await {
        Ok(channel) => channel,
        Err(error) => {
            let _ = evt.send(WorkerOut::Status {
                tab,
                gen,
                state: LinkState::Offline(format!("无法打开会话通道：{error}")),
            });
            return;
        }
    };

    let cols = request.cols.max(20);
    let rows = request.rows.max(5);
    let modes: &[(russh::Pty, u32)] = &[];
    if let Err(error) = channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, modes)
        .await
    {
        let _ = evt.send(WorkerOut::Status {
            tab,
            gen,
            state: LinkState::Offline(format!("无法分配终端：{error}")),
        });
        return;
    }
    if let Err(error) = channel.request_shell(true).await {
        let _ = evt.send(WorkerOut::Status {
            tab,
            gen,
            state: LinkState::Offline(format!("无法启动 shell：{error}")),
        });
        return;
    }

    let _ = evt.send(WorkerOut::Status {
        tab,
        gen,
        state: LinkState::Online,
    });

    let sftp = match tokio::time::timeout(Duration::from_secs(6), open_sftp(&session)).await {
        Ok(Ok(sftp)) => {
            if let Ok(cwd) = sftp.canonicalize(".").await {
                if let Ok(entries) = read_entries(&sftp, &cwd).await {
                    let _ = evt.send(WorkerOut::Dir {
                        tab,
                        gen,
                        path: cwd,
                        entries,
                    });
                }
            }
            Some(Arc::new(tokio::sync::Mutex::new(sftp)))
        }
        Ok(Err(error)) => {
            let _ = evt.send(WorkerOut::Notice {
                tab,
                gen,
                text: format!("SFTP 不可用：{error}"),
                refresh: false,
            });
            None
        }
        Err(_) => {
            let _ = evt.send(WorkerOut::Notice {
                tab,
                gen,
                text: "SFTP 初始化超时，终端仍可使用".to_string(),
                refresh: false,
            });
            None
        }
    };

    let session = Arc::new(tokio::sync::Mutex::new(session));
    let monitor = tokio::spawn(monitor_remote(session.clone(), evt.clone(), tab, gen));

    let end;
    loop {
        let step = tokio::select! {
            msg = channel.wait() => Step::Server(msg),
            cmd = cmd_rx.recv() => Step::Ui(cmd),
        };
        match step {
            Step::Server(None) => {
                end = "远程关闭了连接".to_string();
                break;
            }
            Step::Server(Some(ChannelMsg::Data { data })) => {
                let _ = evt.send(WorkerOut::Data {
                    tab,
                    gen,
                    bytes: bytes_of(&data),
                });
            }
            Step::Server(Some(ChannelMsg::ExtendedData { data, .. })) => {
                let _ = evt.send(WorkerOut::Data {
                    tab,
                    gen,
                    bytes: bytes_of(&data),
                });
            }
            Step::Server(Some(ChannelMsg::ExitStatus { exit_status })) => {
                end = format!("远程进程已退出，状态 {exit_status}");
                break;
            }
            Step::Server(Some(ChannelMsg::Eof | ChannelMsg::Close)) => {
                end = "远程关闭了会话".to_string();
                break;
            }
            Step::Server(Some(_)) => {}
            Step::Ui(None) => {
                end = "已断开".to_string();
                break;
            }
            Step::Ui(Some(SessionCmd::Data(data))) => {
                if channel.data(&data[..]).await.is_err() {
                    end = "发送输入失败".to_string();
                    break;
                }
            }
            Step::Ui(Some(SessionCmd::Resize { cols, rows })) => {
                let _ = channel.window_change(cols.max(1), rows.max(1), 0, 0).await;
            }
            Step::Ui(Some(other)) => spawn_sftp(sftp.clone(), evt.clone(), tab, gen, other),
        }
    }

    drop(channel);
    monitor.abort();
    let session = session.lock().await;
    let _ = session
        .disconnect(Disconnect::ByApplication, &end, "en")
        .await;
    let _ = evt.send(WorkerOut::Status {
        tab,
        gen,
        state: LinkState::Offline(end),
    });
}

enum Step {
    Server(Option<ChannelMsg>),
    Ui(Option<SessionCmd>),
}

async fn authenticate(
    session: &mut client::Handle<ClientHandler>,
    request: &ConnectRequest,
) -> Result<(), String> {
    match &request.auth {
        RuntimeAuth::Password(password) => {
            let password_auth = session
                .authenticate_password(&request.username, password)
                .await
                .map_err(|error| format!("认证失败：{error}"))?;
            if password_auth.success() {
                return Ok(());
            }
            keyboard_interactive(session, &request.username, password).await
        }
        RuntimeAuth::PrivateKey { path, passphrase } => {
            let passphrase = passphrase.as_deref().filter(|value| !value.is_empty());
            let key = russh::keys::load_secret_key(path, passphrase)
                .map_err(|error| format!("无法读取私钥：{error}"))?;
            let hash = session
                .best_supported_rsa_hash()
                .await
                .ok()
                .flatten()
                .flatten();
            let auth = session
                .authenticate_publickey(
                    &request.username,
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(|error| format!("公钥认证失败：{error}"))?;
            if auth.success() {
                Ok(())
            } else {
                Err("公钥认证被拒绝，请检查用户名、私钥和口令".to_string())
            }
        }
    }
}

async fn keyboard_interactive(
    session: &mut client::Handle<ClientHandler>,
    username: &str,
    password: &str,
) -> Result<(), String> {
    let mut response = session
        .authenticate_keyboard_interactive_start(username, None)
        .await
        .map_err(|error| format!("认证失败：{error}"))?;
    for _ in 0..8 {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure { .. } => {
                return Err("认证被拒绝，请检查用户名和密码".to_string());
            }
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers = prompts.into_iter().map(|_| password.to_string()).collect();
                response = session
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(|error| format!("认证失败：{error}"))?;
            }
        }
    }
    Err("键盘交互认证未完成".to_string())
}

async fn open_sftp(session: &client::Handle<ClientHandler>) -> Result<SftpSession, String> {
    let channel = session
        .channel_open_session()
        .await
        .map_err(|error| error.to_string())?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| error.to_string())?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| error.to_string())?;
    sftp.set_timeout(120);
    Ok(sftp)
}

fn spawn_sftp(
    sftp: Option<Arc<tokio::sync::Mutex<SftpSession>>>,
    evt: UnboundedSender<WorkerOut>,
    tab: u64,
    gen: u64,
    cmd: SessionCmd,
) {
    let Some(sftp) = sftp else {
        let _ = evt.send(WorkerOut::Notice {
            tab,
            gen,
            text: "当前会话没有 SFTP".to_string(),
            refresh: false,
        });
        return;
    };
    tokio::spawn(async move {
        let result = {
            let client = sftp.lock().await;
            match cmd {
                SessionCmd::List(path) => read_entries(&client, &path)
                    .await
                    .map(|entries| SftpResult::Dir { path, entries }),
                SessionCmd::Mkdir(path) => client
                    .create_dir(&path)
                    .await
                    .map(|()| SftpResult::Done(format!("已创建目录 {path}")))
                    .map_err(|error| error.to_string()),
                SessionCmd::Remove { path, dir } => {
                    let op = if dir {
                        client.remove_dir(&path).await
                    } else {
                        client.remove_file(&path).await
                    };
                    op.map(|()| SftpResult::Done(format!("已删除 {path}")))
                        .map_err(|error| error.to_string())
                }
                SessionCmd::Download { remote, local } => {
                    download(&client, &remote, &local, &evt, tab, gen).await
                }
                SessionCmd::Upload { local, remote } => {
                    upload(&client, &local, &remote, &evt, tab, gen).await
                }
                SessionCmd::Data(_) | SessionCmd::Resize { .. } => {
                    Ok(SftpResult::Done(String::new()))
                }
            }
        };
        match result {
            Ok(SftpResult::Dir { path, entries }) => {
                let _ = evt.send(WorkerOut::Dir {
                    tab,
                    gen,
                    path,
                    entries,
                });
            }
            Ok(SftpResult::Done(text)) => {
                if !text.is_empty() {
                    let _ = evt.send(WorkerOut::Notice {
                        tab,
                        gen,
                        text,
                        refresh: true,
                    });
                }
            }
            Err(error) => {
                let _ = evt.send(WorkerOut::Notice {
                    tab,
                    gen,
                    text: error,
                    refresh: false,
                });
            }
        }
    });
}

enum SftpResult {
    Dir {
        path: String,
        entries: Vec<RemoteEntry>,
    },
    Done(String),
}

async fn read_entries(sftp: &SftpSession, path: &str) -> Result<Vec<RemoteEntry>, String> {
    let listed = sftp.read_dir(path).await.map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for entry in listed {
        let name = entry.file_name().to_string();
        if name == "." || name == ".." || name.is_empty() {
            continue;
        }
        let kind = entry.file_type();
        let is_dir = kind.is_dir();
        let is_link = kind.is_symlink();
        let size = entry.metadata().size.unwrap_or(0);
        entries.push(RemoteEntry {
            name,
            is_dir,
            is_link,
            size,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

async fn download(
    sftp: &SftpSession,
    remote: &str,
    local: &PathBuf,
    evt: &UnboundedSender<WorkerOut>,
    tab: u64,
    gen: u64,
) -> Result<SftpResult, String> {
    if let Some(parent) = local.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("无法创建本地目录：{error}"))?;
    }
    let total = sftp
        .metadata(remote)
        .await
        .ok()
        .and_then(|meta| meta.size)
        .unwrap_or(0);
    let mut remote_file = sftp
        .open(remote)
        .await
        .map_err(|error| format!("无法打开远程文件：{error}"))?;
    let mut local_file = tokio::fs::File::create(local)
        .await
        .map_err(|error| format!("无法创建本地文件：{error}"))?;
    let mut meter = Meter::new(evt, tab, gen, false, file_label(remote), total);
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let read = remote_file
            .read(&mut buf)
            .await
            .map_err(|error| format!("下载失败：{error}"))?;
        if read == 0 {
            break;
        }
        local_file
            .write_all(&buf[..read])
            .await
            .map_err(|error| format!("写入本地文件失败：{error}"))?;
        meter.add(read as u64);
    }
    meter.finish();
    Ok(SftpResult::Done(format!(
        "已下载到 {}（{}，平均 {}/s）",
        local.display(),
        format_bytes(meter.done),
        format_bytes(meter.average()),
    )))
}

async fn upload(
    sftp: &SftpSession,
    local: &PathBuf,
    remote: &str,
    evt: &UnboundedSender<WorkerOut>,
    tab: u64,
    gen: u64,
) -> Result<SftpResult, String> {
    let total = tokio::fs::metadata(local)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);
    let mut local_file = tokio::fs::File::open(local)
        .await
        .map_err(|error| format!("无法读取本地文件：{error}"))?;
    let mut remote_file = sftp
        .create(remote)
        .await
        .map_err(|error| format!("无法创建远程文件：{error}"))?;
    let name = local
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("upload");
    let mut meter = Meter::new(evt, tab, gen, true, name.to_string(), total);
    let mut buf = vec![0_u8; 64 * 1024];
    loop {
        let read = local_file
            .read(&mut buf)
            .await
            .map_err(|error| format!("读取本地文件失败：{error}"))?;
        if read == 0 {
            break;
        }
        remote_file
            .write_all(&buf[..read])
            .await
            .map_err(|error| format!("上传失败：{error}"))?;
        meter.add(read as u64);
    }
    meter.finish();
    Ok(SftpResult::Done(format!(
        "已上传 {remote}（{}，平均 {}/s）",
        format_bytes(meter.done),
        format_bytes(meter.average()),
    )))
}

struct Meter<'a> {
    evt: &'a UnboundedSender<WorkerOut>,
    tab: u64,
    gen: u64,
    upload: bool,
    name: String,
    total: u64,
    done: u64,
    speed: u64,
    started: Instant,
    window_at: Instant,
    window_bytes: u64,
    last_emit: Instant,
}

impl<'a> Meter<'a> {
    fn new(
        evt: &'a UnboundedSender<WorkerOut>,
        tab: u64,
        gen: u64,
        upload: bool,
        name: String,
        total: u64,
    ) -> Self {
        let now = Instant::now();
        let meter = Self {
            evt,
            tab,
            gen,
            upload,
            name,
            total,
            done: 0,
            speed: 0,
            started: now,
            window_at: now,
            window_bytes: 0,
            last_emit: now,
        };
        meter.emit();
        meter
    }

    fn add(&mut self, n: u64) {
        self.done += n;
        let now = Instant::now();
        let elapsed = now.duration_since(self.window_at);
        if elapsed >= Duration::from_millis(250) {
            let delta = self.done.saturating_sub(self.window_bytes);
            self.speed = (delta as f64 / elapsed.as_secs_f64()) as u64;
            self.window_at = now;
            self.window_bytes = self.done;
        }
        if now.duration_since(self.last_emit) >= Duration::from_millis(200) {
            self.emit();
            self.last_emit = now;
        }
    }

    fn finish(&mut self) {
        self.speed = self.average();
        self.emit();
    }

    fn average(&self) -> u64 {
        let elapsed = self.started.elapsed().as_secs_f64().max(0.05);
        (self.done as f64 / elapsed) as u64
    }

    fn emit(&self) {
        let _ = self.evt.send(WorkerOut::Transfer {
            tab: self.tab,
            gen: self.gen,
            transfer: Transfer {
                upload: self.upload,
                name: self.name.clone(),
                done: self.done,
                total: self.total,
                speed: self.speed,
            },
        });
    }
}

const STATS_CMD: &str = "sh -c 'printf \"@@STAT@@\\n\"; head -1 /proc/stat; printf \"@@MEM@@\\n\"; cat /proc/meminfo; printf \"@@UP@@\\n\"; cat /proc/uptime; printf \"@@NET@@\\n\"; cat /proc/net/dev; printf \"@@DF@@\\n\"; df -PT 2>/dev/null || df -P; printf \"@@WHO@@\\n\"; who 2>/dev/null | wc -l; printf \"@@HOST@@\\n\"; hostname; printf \"@@END@@\\n\"'";

struct RawSample {
    at: Instant,
    cpu_total: u64,
    cpu_idle: u64,
    rx: u64,
    tx: u64,
}

async fn monitor_remote(
    session: Arc<tokio::sync::Mutex<client::Handle<ClientHandler>>>,
    evt: UnboundedSender<WorkerOut>,
    tab: u64,
    gen: u64,
) {
    let mut previous: Option<RawSample> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let guard = session.lock().await;
        if guard.is_closed() {
            break;
        }
        let output = match tokio::time::timeout(
            Duration::from_secs(4),
            run_command(&guard, STATS_CMD),
        )
        .await
        {
            Ok(Ok(output)) => output,
            _ => continue,
        };
        drop(guard);
        let now = Instant::now();
        let Some((stats, sample)) = parse_stats(&output, previous.as_ref(), now) else {
            continue;
        };
        previous = Some(sample);
        if evt
            .send(WorkerOut::Stats { tab, gen, stats })
            .is_err()
        {
            break;
        }
    }
}

async fn run_command(
    session: &client::Handle<ClientHandler>,
    command: &str,
) -> Result<String, String> {
    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|error| error.to_string())?;
    channel
        .exec(true, command)
        .await
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { data }) => out.extend(bytes_of(&data)),
            Some(ChannelMsg::ExtendedData { .. }) => {}
            Some(ChannelMsg::Eof) => {}
            Some(ChannelMsg::ExitStatus { .. }) => {}
            Some(ChannelMsg::Close) | None => break,
            Some(_) => {}
        }
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn parse_stats(text: &str, previous: Option<&RawSample>, now: Instant) -> Option<(RemoteStats, RawSample)> {
    let (cpu_total, cpu_idle) = parse_cpu(section(text, "STAT"))?;
    let mem_total = mem_bytes(section(text, "MEM"), "MemTotal:")?;
    let available = mem_bytes(section(text, "MEM"), "MemAvailable:").or_else(|| {
        let free = mem_bytes(section(text, "MEM"), "MemFree:")?;
        let buffers = mem_bytes(section(text, "MEM"), "Buffers:").unwrap_or(0);
        let cached = mem_bytes(section(text, "MEM"), "Cached:").unwrap_or(0);
        Some(free + buffers + cached)
    })?;
    let uptime_secs = section(text, "UP")
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;
    let (rx, tx) = parse_net(section(text, "NET"));
    let sample = RawSample {
        at: now,
        cpu_total,
        cpu_idle,
        rx,
        tx,
    };
    let (cpu_percent, net_rx, net_tx) = if let Some(prev) = previous {
        let elapsed = now.duration_since(prev.at).as_secs_f64().max(0.2);
        let total_delta = cpu_total.saturating_sub(prev.cpu_total);
        let idle_delta = cpu_idle.saturating_sub(prev.cpu_idle);
        let cpu_percent = if total_delta == 0 {
            0.0
        } else {
            (1.0 - idle_delta as f32 / total_delta as f32).clamp(0.0, 1.0) * 100.0
        };
        let net_rx = ((rx.saturating_sub(prev.rx)) as f64 / elapsed) as u64;
        let net_tx = ((tx.saturating_sub(prev.tx)) as f64 / elapsed) as u64;
        (cpu_percent, net_rx, net_tx)
    } else {
        (0.0, 0, 0)
    };
    let sessions = section(text, "WHO")
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let host = section(text, "HOST")
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("主机")
        .to_string();
    let disks = section(text, "DF").lines().filter_map(parse_df_line).collect();
    Some((
        RemoteStats {
            host,
            cpu_percent,
            mem_used: mem_total.saturating_sub(available.min(mem_total)),
            mem_total,
            net_rx,
            net_tx,
            uptime_secs,
            sessions,
            disks,
        },
        sample,
    ))
}

fn section<'a>(text: &'a str, name: &str) -> &'a str {
    let marker = format!("@@{name}@@");
    let Some(start) = text.find(&marker) else {
        return "";
    };
    let body = text[start + marker.len()..].trim_start_matches(['\r', '\n']);
    match body.find("@@") {
        Some(end) => &body[..end],
        None => body,
    }
}

fn parse_cpu(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let rest = line.strip_prefix("cpu")?;
    if rest.starts_with(|ch: char| ch.is_ascii_digit()) {
        return None;
    }
    let rest = rest.trim_start();
    let nums: Vec<u64> = rest
        .split_whitespace()
        .filter_map(|value| value.parse().ok())
        .collect();
    if nums.len() < 4 {
        return None;
    }
    let total = nums.iter().sum();
    let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
    Some((total, idle))
}

fn mem_bytes(text: &str, key: &str) -> Option<u64> {
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix(key) else {
            continue;
        };
        let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        return Some(kb.saturating_mul(1024));
    }
    None
}

fn parse_net(text: &str) -> (u64, u64) {
    let mut rx = 0;
    let mut tx = 0;
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name == "lo" {
            continue;
        }
        let nums: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|value| value.parse().ok())
            .collect();
        if nums.len() >= 9 {
            rx += nums[0];
            tx += nums[8];
        }
    }
    (rx, tx)
}

fn parse_df_line(line: &str) -> Option<DiskUsage> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let pct_at = parts.iter().position(|part| {
        let Some(number) = part.strip_suffix('%') else {
            return false;
        };
        !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit())
    })?;
    if pct_at + 1 >= parts.len() {
        return None;
    }
    let percent = parts[pct_at].trim_end_matches('%').parse().ok()?;
    let mount = parts[pct_at + 1..].join(" ");
    if mount.is_empty() || mount == "on" {
        return None;
    }
    if let Some(kind) = parts.get(1) {
        if kind.chars().all(|ch| ch.is_ascii_alphabetic())
            && matches!(
                *kind,
                "tmpfs"
                    | "devtmpfs"
                    | "squashfs"
                    | "overlay"
                    | "efivarfs"
                    | "autofs"
                    | "proc"
                    | "sysfs"
                    | "cgroup2"
                    | "devpts"
                    | "nsfs"
                    | "ramfs"
            )
        {
            return None;
        }
    }
    if mount.starts_with("/snap") || mount.starts_with("/run") {
        return None;
    }
    Some(DiskUsage { mount, percent })
}

fn file_label(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn format_bytes(bytes: u64) -> String {
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

fn fingerprint_of(key: &PublicKeyOrCertificate) -> String {
    public_fingerprint(&key.public_key())
}

fn public_fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

fn bytes_of(data: &[u8]) -> Vec<u8> {
    data.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_snapshot() {
        let text = r#"
@@STAT@@
cpu  100 0 50 800 50 0 0 0 0 0
@@MEM@@
MemTotal:       33554432 kB
MemAvailable:   16777216 kB
@@UP@@
1123200.50 100.0
@@NET@@
    lo: 10 0 0 0 0 0 0 0 10 0 0 0 0 0 0 0
  eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0
@@DF@@
Filesystem Type 1024-blocks Used Available Capacity Mounted on
/dev/sda2 ext4 100 8 92 8% /
tmpfs tmpfs 100 1 99 1% /run
/dev/sda1 vfat 100 10 90 10% /boot/efi
@@WHO@@
2
@@HOST@@
xiaofujia-bak
@@END@@
"#;
        let now = Instant::now();
        let (stats, sample) = parse_stats(text, None, now).expect("stats");
        assert_eq!(stats.host, "xiaofujia-bak");
        assert_eq!(stats.cpu_percent, 0.0);
        assert_eq!(stats.mem_total, 33554432 * 1024);
        assert_eq!(stats.mem_used, stats.mem_total / 2);
        assert_eq!(stats.uptime_secs, 1_123_200);
        assert_eq!(stats.sessions, 2);
        assert_eq!(stats.disks.len(), 2);
        assert_eq!(stats.disks[0].mount, "/");
        assert_eq!(stats.disks[0].percent, 8);
        assert_eq!(sample.rx, 1000);
        assert_eq!(sample.tx, 2000);

        let later = text.replace("cpu  100 0 50 800 50", "cpu  200 0 100 900 50");
        let later = later.replace("eth0: 1000", "eth0: 3000").replace("2000 0", "4000 0");
        let (next, _) = parse_stats(&later, Some(&sample), now + Duration::from_secs(2)).unwrap();
        assert!(next.cpu_percent > 40.0 && next.cpu_percent < 80.0);
        assert_eq!(next.net_rx, 1000);
        assert_eq!(next.net_tx, 1000);
    }
}
