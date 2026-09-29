use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client;
use russh::ChannelMsg;
use tokio::sync::mpsc::UnboundedSender;

use super::session::ClientHandler;
use super::*;

pub(crate) const STATS_CMD: &str = "sh -c 'printf \"@@STAT@@\\n\"; head -1 /proc/stat; printf \"@@MEM@@\\n\"; cat /proc/meminfo; printf \"@@UP@@\\n\"; cat /proc/uptime; printf \"@@NET@@\\n\"; cat /proc/net/dev; printf \"@@DF@@\\n\"; df -PT 2>/dev/null || df -P; printf \"@@WHO@@\\n\"; who 2>/dev/null | wc -l; printf \"@@HOST@@\\n\"; hostname; printf \"@@END@@\\n\"'";

pub(crate) struct RawSample {
    at: Instant,
    cpu_total: u64,
    cpu_idle: u64,
    rx: u64,
    tx: u64,
}

pub(crate) async fn monitor_remote(
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

pub(crate) async fn run_command(
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

pub(crate) fn parse_stats(text: &str, previous: Option<&RawSample>, now: Instant) -> Option<(RemoteStats, RawSample)> {
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

pub(crate) fn section<'a>(text: &'a str, name: &str) -> &'a str {
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

pub(crate) fn parse_cpu(text: &str) -> Option<(u64, u64)> {
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

pub(crate) fn mem_bytes(text: &str, key: &str) -> Option<u64> {
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix(key) else {
            continue;
        };
        let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        return Some(kb.saturating_mul(1024));
    }
    None
}

pub(crate) fn parse_net(text: &str) -> (u64, u64) {
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

pub(crate) fn parse_df_line(line: &str) -> Option<DiskUsage> {
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
