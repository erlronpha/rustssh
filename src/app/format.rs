use crate::ssh;

pub(crate) fn ellipsize(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub(crate) fn short_text(value: &str, max: usize) -> String {
    ellipsize(value, max)
}

pub(crate) fn transfer_label(transfer: &ssh::Transfer) -> String {
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

pub(crate) fn human_size(bytes: u64) -> String {
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
