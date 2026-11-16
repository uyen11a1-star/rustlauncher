//! Tải, giải nén và chuẩn bị JRE Android (arm64). Tự viết: tải có tiến trình, giải xz, giải tar, kiểm tra.
use std::{
    fs,
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{atomic::Ordering::Relaxed, Arc},
    time::Duration,
};

use crate::{
    jvm,
    mc::{Res, Shared},
    tarx,
};

pub const ARCH_NAME: &str = "arm64";
pub const OS_ARCH: &str = "aarch64";

/// Nguồn JRE dựng sẵn cho Android (một trong những phần bất khả thi tự làm).
pub fn urls(major: u32) -> Vec<String> {
    let base = "https://github.com/AngelAuraMC/angelauramc-openjdk-build/releases/download";
    vec![
        format!("{base}/download_jre{major}/jre{major}-android-{ARCH_NAME}.tar.xz"),
        format!("{base}/download/jre{major}-android-{ARCH_NAME}.tar.xz"),
    ]
}

pub fn runtime_dir(root: &Path, major: u32) -> PathBuf {
    root.join("runtimes").join(format!("jre{major}"))
}

/// JRE đã cài xong (có dấu `.ok` và có `release`)?
pub fn installed(root: &Path, major: u32) -> Option<PathBuf> {
    let dir = runtime_dir(root, major);
    if dir.join(".ok").exists() {
        jvm::find_java_home(&dir)
    } else {
        None
    }
}

async fn download(client: &reqwest::Client, url: &str, dest: &Path, shared: &Shared) -> Res<()> {
    let mut resp = client.get(url).send().await?.error_for_status()?;
    shared.bytes_total.store(resp.content_length().unwrap_or(0), Relaxed);
    shared.bytes_done.store(0, Relaxed);
    let mut f = BufWriter::with_capacity(1 << 20, fs::File::create(dest)?);
    let mut got = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        f.write_all(&chunk)?;
        got += chunk.len() as u64;
        shared.bytes_done.store(got, Relaxed);
    }
    f.flush()?;
    Ok(())
}

fn unxz(src: &Path, dst: &Path) -> Res<()> {
    let mut input = BufReader::with_capacity(1 << 20, fs::File::open(src)?);
    let mut out = BufWriter::with_capacity(1 << 20, fs::File::create(dst)?);
    lzma_rs::xz_decompress(&mut input, &mut out).map_err(|e| format!("giải xz lỗi: {e:?}"))?;
    out.flush()?;
    Ok(())
}

/// Cài JRE `major` vào `root/runtimes/jre{major}`. Trả về java_home.
pub async fn install(root: &Path, major: u32, shared: &Arc<Shared>, log: &(dyn Fn(String) + Send + Sync)) -> Res<PathBuf> {
    if let Some(h) = installed(root, major) {
        log(format!("JRE {major} đã có sẵn, bỏ qua tải."));
        return Ok(h);
    }
    let cache = root.join("cache");
    fs::create_dir_all(&cache)?;
    let xz = cache.join(format!("jre{major}.tar.xz"));
    let tar = cache.join(format!("jre{major}.tar"));

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .user_agent("RustLauncher/0.1")
        .build()?;

    shared.reset_progress();
    shared.set_phase(&format!("Đang tải JRE {major}..."));
    let mut last_err: Option<String> = None;
    let mut ok = false;
    for u in urls(major) {
        log(format!("Tải {u}"));
        match download(&client, &u, &xz, shared).await {
            Ok(()) => {
                ok = true;
                break;
            }
            Err(e) => {
                log(format!("  lỗi: {e}"));
                last_err = Some(e.to_string());
            }
        }
    }
    if !ok {
        return Err(format!("không tải được JRE {major}: {}", last_err.unwrap_or_default()).into());
    }
    log(format!("Đã tải {} MB.", shared.bytes_done.load(Relaxed) / 1_000_000));

    shared.reset_progress();
    shared.set_phase("Đang giải nén xz...");
    let (xz2, tar2) = (xz.clone(), tar.clone());
    tokio::task::spawn_blocking(move || unxz(&xz2, &tar2)).await??;
    let _ = fs::remove_file(&xz);

    shared.set_phase("Đang giải nén tar...");
    let dir = runtime_dir(root, major);
    let tmp = dir.with_extension("tmp");
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.parent().unwrap())?;
    let (tar3, tmp2) = (tar.clone(), tmp.clone());
    let sh = shared.clone();
    let n = tokio::task::spawn_blocking(move || -> std::io::Result<usize> {
        let f = BufReader::with_capacity(1 << 20, fs::File::open(&tar3)?);
        tarx::extract(f, &tmp2, |_, c| sh.files_done.store(c, Relaxed))
    })
    .await??;
    let _ = fs::remove_file(&tar);
    log(format!("Giải nén {n} mục."));
    fs::rename(&tmp, &dir)?;

    let home = jvm::find_java_home(&dir).ok_or("giải nén xong nhưng không thấy file `release` của JRE")?;
    let lib = jvm::pick_lib_dir(&home, OS_ARCH);
    jvm::fix_freetype(&lib)?;
    if let Some((ver, arch)) = jvm::read_release(&home) {
        log(format!("JRE: Java {ver}, kiến trúc {arch}"));
        if arch != OS_ARCH {
            return Err(format!("JRE sai kiến trúc ({arch}), cần {OS_ARCH}").into());
        }
    }
    fs::write(dir.join(".ok"), b"1")?;
    Ok(home)
}
