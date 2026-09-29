use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::sftp::{open_sftp, read_entries, spawn_sftp};
use super::stats::monitor_remote;
use super::*;

pub(crate) struct ClientHandler {
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

pub(crate) async fn run_session(
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

pub(crate) enum Step {
    Server(Option<ChannelMsg>),
    Ui(Option<SessionCmd>),
}

pub(crate) async fn authenticate(
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

pub(crate) async fn keyboard_interactive(
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
