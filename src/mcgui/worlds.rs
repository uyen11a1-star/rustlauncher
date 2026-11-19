//! Quét thư mục world và đọc level.dat để lấy thông tin hiển thị (tên, thời điểm chơi, chế độ, phiên bản).
use std::path::PathBuf;

use crate::{inflate, nbt};

#[derive(Clone)]
pub struct WorldEntry {
    pub name: String,
    pub folder: String,
    pub dir: PathBuf,
    /// mili giây Unix
    pub last_played: i64,
    pub version: String,
    pub game_mode: i32,
    pub hardcore: bool,
    pub cheats: bool,
}

fn read_one(dir: &std::path::Path) -> Option<WorldEntry> {
    let folder = dir.file_name()?.to_string_lossy().into_owned();
    let bytes = std::fs::read(dir.join("level.dat")).ok();
    let mut e = WorldEntry { name: folder.clone(), folder, dir: dir.to_path_buf(), last_played: 0, version: String::new(), game_mode: 0, hardcore: false, cheats: false };
    if let Some(b) = bytes {
        if let Ok(raw) = inflate::gzip_any(&b, 64 << 20) {
            if let Ok(root) = nbt::parse(&raw) {
                if let Some(d) = root.get("Data") {
                    if let Some(n) = d.get("LevelName").and_then(|v| v.as_str()) {
                        e.name = n.to_string();
                    }
                    e.last_played = d.get("LastPlayed").and_then(|v| v.as_i64()).unwrap_or(0);
                    e.version = d.get("Version").and_then(|v| v.get("Name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    e.game_mode = d.get("GameType").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    e.hardcore = d.get("hardcore").and_then(|v| v.as_i64()).unwrap_or(0) != 0;
                    e.cheats = d.get("allowCommands").and_then(|v| v.as_i64()).unwrap_or(0) != 0;
                }
            }
        }
    } else if !dir.join("region").is_dir() {
        return None;
    }
    Some(e)
}

/// Mọi thư mục con (hoặc chính thư mục gốc) có level.dat hoặc region/, mới chơi nhất lên đầu.
pub fn scan(base: &str) -> Result<Vec<WorldEntry>, String> {
    let rd = std::fs::read_dir(base).map_err(|e| format!("{e}"))?;
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.insert(0, PathBuf::from(base));
    let mut out: Vec<WorldEntry> = Vec::new();
    for d in dirs {
        if d.join("level.dat").is_file() || d.join("region").is_dir() {
            if let Some(e) = read_one(&d) {
                out.push(e);
            }
        }
    }
    out.sort_by(|a, b| b.last_played.cmp(&a.last_played));
    Ok(out)
}

/// "11/18/26, 10:30 AM" theo giờ UTC cộng `tz_min` phút (kiểu DateFormat SHORT của en_US).
pub fn fmt_date(ms: i64, tz_min: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let secs = ms / 1000 + tz_min * 60;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // ngày dân sự (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let (h, mi) = (rem / 3600, rem % 3600 / 60);
    let (h12, ap) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" });
    format!("{m}/{d}/{:02}, {h12}:{mi:02} {ap}", y % 100)
}
