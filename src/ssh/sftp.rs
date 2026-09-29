use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client;
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedSender;

use super::session::ClientHandler;
use super::*;

pub(crate) async fn open_sftp(session: &client::Handle<ClientHandler>) -> Result<SftpSession, String> {
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

pub(crate) fn spawn_sftp(
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

pub(crate) enum SftpResult {
    Dir {
        path: String,
        entries: Vec<RemoteEntry>,
    },
    Done(String),
}

pub(crate) async fn read_entries(sftp: &SftpSession, path: &str) -> Result<Vec<RemoteEntry>, String> {
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

pub(crate) async fn download(
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

pub(crate) async fn upload(
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

pub(crate) struct Meter<'a> {
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
