use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub group: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: Auth,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Auth {
    Password { saved: Option<String> },
    PrivateKey {
        path: String,
        passphrase: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickTarget {
    pub user: Option<String>,
    pub host: String,
    pub port: u16,
}

pub fn default_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "root".to_string())
}

pub fn app_dir() -> PathBuf {
    dirs::config_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustssh")
}

pub fn load_profiles() -> Result<Vec<Profile>, String> {
    let path = app_dir().join("sessions.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("无法读取会话文件：{e}"))?;
    serde_json::from_str(&text).map_err(|e| {
        let backup = path.with_extension("json.bak");
        let _ = fs::copy(&path, backup);
        format!("会话文件已损坏，已备份为 sessions.json.bak：{e}")
    })
}

pub fn save_profiles(profiles: &[Profile]) -> Result<(), String> {
    let dir = app_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录：{e}"))?;
    let text = serde_json::to_string_pretty(profiles).map_err(|e| e.to_string())?;
    fs::write(dir.join("sessions.json"), text).map_err(|e| format!("无法保存会话：{e}"))
}

pub fn load_known_hosts() -> BTreeMap<String, String> {
    let path = app_dir().join("known_hosts.json");
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_known_hosts(hosts: &BTreeMap<String, String>) -> Result<(), String> {
    let dir = app_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录：{e}"))?;
    let text = serde_json::to_string_pretty(hosts).map_err(|e| e.to_string())?;
    fs::write(dir.join("known_hosts.json"), text).map_err(|e| format!("无法保存主机密钥：{e}"))
}

pub fn host_key(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// Accepts `user@host`, `user@host:port`, `host:port`, `ssh://user@host:port`,
/// and `[ipv6]:port`.
pub fn parse_target(input: &str) -> Option<QuickTarget> {
    let input = input.trim();
    let input = input
        .strip_prefix("ssh://")
        .or_else(|| input.strip_prefix("SSH://"))
        .unwrap_or(input);
    if input.is_empty() {
        return None;
    }

    let (user, hostport) = match input.rsplit_once('@') {
        Some((user, hostport)) if !user.is_empty() && !hostport.is_empty() => {
            (Some(user.to_string()), hostport)
        }
        _ => (None, input),
    };

    if let Some(rest) = hostport.strip_prefix('[') {
        let (addr, after) = rest.split_once(']')?;
        if addr.is_empty() {
            return None;
        }
        let port = match after {
            "" => 22,
            port => port.strip_prefix(':')?.parse().ok()?,
        };
        return Some(QuickTarget {
            user,
            host: addr.to_string(),
            port,
        });
    }

    let (host, port) = match hostport.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty() && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            (host, port.parse().ok()?)
        }
        _ => (hostport, 22),
    };
    if host.is_empty() || host.contains(' ') {
        return None;
    }
    Some(QuickTarget {
        user,
        host: host.to_string(),
        port,
    })
}

pub fn join_remote(dir: &str, name: &str) -> String {
    if name == ".." {
        return parent_remote(dir);
    }
    if dir.is_empty() || dir == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", dir.trim_end_matches('/'))
    }
}

pub fn parent_remote(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    match trimmed.rsplit_once('/') {
        Some(("", _)) => "/".to_string(),
        Some((parent, _)) if parent.is_empty() => "/".to_string(),
        Some((parent, _)) => parent.to_string(),
        None => "/".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_targets() {
        assert_eq!(
            parse_target("root@10.0.0.8"),
            Some(QuickTarget {
                user: Some("root".into()),
                host: "10.0.0.8".into(),
                port: 22,
            })
        );
        assert_eq!(
            parse_target("ssh://alice@example.com:2222"),
            Some(QuickTarget {
                user: Some("alice".into()),
                host: "example.com".into(),
                port: 2222,
            })
        );
        assert_eq!(
            parse_target("[2001:db8::1]:2200"),
            Some(QuickTarget {
                user: None,
                host: "2001:db8::1".into(),
                port: 2200,
            })
        );
        assert!(parse_target("").is_none());
        assert!(parse_target("has space").is_none());
    }

    #[test]
    fn joins_remote_paths() {
        assert_eq!(join_remote("/var", "log"), "/var/log");
        assert_eq!(join_remote("/", "etc"), "/etc");
        assert_eq!(join_remote("/var/log", ".."), "/var");
        assert_eq!(parent_remote("/"), "/");
    }
}
