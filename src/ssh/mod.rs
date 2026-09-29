use std::collections::HashMap;
use std::path::PathBuf;

use iced::futures::SinkExt;
use iced::futures::Stream;
use russh::keys::{HashAlg, PublicKey, PublicKeyOrCertificate};
use tokio::sync::mpsc::UnboundedSender;

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

pub(crate) enum SessionCmd {
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

mod session;
mod sftp;
mod stats;

use session::run_session;

pub(crate) fn file_label(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

pub(crate) fn format_bytes(bytes: u64) -> String {
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

pub(crate) fn fingerprint_of(key: &PublicKeyOrCertificate) -> String {
    public_fingerprint(&key.public_key())
}

fn public_fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

pub(crate) fn bytes_of(data: &[u8]) -> Vec<u8> {
    data.to_vec()
}

