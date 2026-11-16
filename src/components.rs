//! Tải thành phần native dựng sẵn (LWJGL cho Android, OpenAL) từ APK Amethyst bằng HTTP Range:
//! chỉ lấy đúng vài file cần, không tải cả APK 150 MB.
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{atomic::Ordering::Relaxed, Arc},
    time::Duration,
};

use crate::{
    mc::{Res, Shared},
    zipr,
};

pub const APK_URL: &str = "https://github.com/AngelAuraMC/Amethyst-Android/releases/download/1.1.7/Amethyst.apk";
/// Các module LWJGL không cần cho Minecraft (nặng).
const SKIP: [&str; 4] = ["shaderc", "nanovg", "vma", "spvc"];

async fn get_range(client: &reqwest::Client, range: String) -> Res<Vec<u8>> {
    let resp = client.get(APK_URL).header(reqwest::header::RANGE, range).send().await?;
    let st = resp.status();
    if st.as_u16() != 206 {
        return Err(format!("máy chủ không trả 206 cho Range (HTTP {st})").into());
    }
    Ok(resp.bytes().await?.to_vec())
}

pub fn natives_dir(root: &Path, ver: &str) -> PathBuf {
    root.join("components").join(format!("lwjgl-{ver}"))
}

/// Đảm bảo có natives LWJGL `ver` (vd "3.3.3") trong `root/components/lwjgl-<ver>`. Trả về thư mục đó.
pub async fn ensure(root: &Path, ver: &str, shared: &Arc<Shared>, log: &(dyn Fn(String) + Send + Sync)) -> Res<PathBuf> {
    let dir = natives_dir(root, ver);
    if dir.join(".ok").exists() {
        log(format!("Natives LWJGL {ver} đã có sẵn."));
        return Ok(dir);
    }
    fs::create_dir_all(&dir)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .user_agent("RustLauncher/0.1")
        .build()?;

    shared.reset_progress();
    shared.set_phase("Đang tải thư viện native (LWJGL)...");
    log("Đọc mục lục APK Amethyst (HTTP Range)...".into());
    let tail = get_range(&client, "bytes=-262144".into()).await?;
    let (cd_off, cd_size) = zipr::find_cd(&tail).ok_or("không thấy EOCD của APK")?;
    let cd = get_range(&client, format!("bytes={}-{}", cd_off, cd_off + cd_size - 1)).await?;
    let entries = zipr::parse_central(&cd)?;
    log(format!("APK có {} mục.", entries.len()));

    let prefix = format!("assets/components/lwjgl-{ver}-natives/arm64-v8a/");
    let mut want: Vec<(&zipr::Entry, String)> = Vec::new();
    let mut lwjgl_count = 0;
    for e in &entries {
        if let Some(rest) = e.name.strip_prefix(&prefix) {
            if !rest.is_empty() && !rest.contains('/') && !SKIP.iter().any(|s| rest.contains(s)) {
                want.push((e, rest.to_string()));
                lwjgl_count += 1;
            }
        } else if e.name == "lib/arm64-v8a/libopenal.so" {
            want.push((e, "libopenal.so".into()));
        } else if e.name == "lib/arm64-v8a/libc++_shared.so" {
            want.push((e, "libc++_shared.so".into()));
        }
    }
    if lwjgl_count == 0 {
        let comps: BTreeSet<String> = entries
            .iter()
            .filter_map(|e| e.name.strip_prefix("assets/components/"))
            .filter_map(|r| r.split('/').next())
            .map(String::from)
            .collect();
        return Err(format!("APK không có natives LWJGL {ver}. Các component có: {}", comps.into_iter().collect::<Vec<_>>().join(", ")).into());
    }

    shared.files_total.store(want.len(), Relaxed);
    shared.bytes_total.store(want.iter().map(|(e, _)| e.size).sum(), Relaxed);
    let mut done_bytes = 0u64;
    for (i, (e, name)) in want.iter().enumerate() {
        log(format!("Tải {name} ({} KB)", e.size / 1024));
        // 30 byte header + tên/extra: lấy dư 1 KB, máy chủ cắt ở cuối file nếu vượt.
        let buf = get_range(&client, format!("bytes={}-{}", e.local_off, e.local_off + 30 + 1024 + e.comp_size - 1)).await?;
        let data = zipr::entry_data(&buf, e)?;
        let tmp = dir.join(format!("{name}.part"));
        fs::write(&tmp, &data)?;
        fs::rename(&tmp, dir.join(name))?;
        done_bytes += e.size;
        shared.bytes_done.store(done_bytes, Relaxed);
        shared.files_done.store(i + 1, Relaxed);
    }
    fs::write(dir.join(".ok"), b"1")?;
    log(format!("Đã cài {} file native vào {}", want.len(), dir.display()));
    Ok(dir)
}
