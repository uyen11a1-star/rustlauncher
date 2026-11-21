//! Tải danh sách phiên bản và client.jar từ Mojang (có kiểm SHA-1, bỏ qua file đã có).
#![allow(dead_code)]

use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed},
        Arc, Mutex,
    },
    time::Duration,
};

use reqwest::Client;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use tokio::{fs, io::AsyncWriteExt};

pub type Res<T> = Result<T, Box<dyn Error + Send + Sync>>;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const OBJECTS_URL: &str = "https://resources.download.minecraft.net";
const CONCURRENCY: usize = 48;

// ---------- Dữ liệu dùng chung giữa luồng tải và UI ----------

#[derive(Default)]
pub struct Shared {
    pub versions: Mutex<Vec<VersionEntry>>,
    pub phase: Mutex<String>,
    pub error: Mutex<Option<String>>,
    pub running: AtomicBool,
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    pub files_done: AtomicUsize,
    pub files_total: AtomicUsize,
    /// ghi chú của luồng tải để luồng chính đẩy vào nhật ký (vd lỗi tải panorama)
    pub notes: Mutex<Vec<String>>,
}

impl Shared {
    pub fn set_phase(&self, s: &str) {
        *self.phase.lock().unwrap() = s.to_owned();
    }

    pub fn reset_progress(&self) {
        self.bytes_done.store(0, Relaxed);
        self.bytes_total.store(0, Relaxed);
        self.files_done.store(0, Relaxed);
        self.files_total.store(0, Relaxed);
    }
}

// ---------- Cấu trúc JSON của Mojang ----------

#[derive(Deserialize)]
struct VersionManifest {
    versions: Vec<VersionEntry>,
}

#[derive(Deserialize, Clone)]
pub struct VersionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub sha1: String,
}

#[derive(Deserialize)]
pub struct VersionMeta {
    pub downloads: Downloads,
    #[serde(rename = "assetIndex", default)]
    pub asset_index: Option<AssetIndexRef>,
}

#[derive(Deserialize, Clone)]
pub struct AssetIndexRef {
    pub id: String,
    pub url: String,
    pub sha1: String,
    pub size: u64,
}

#[derive(Deserialize)]
pub struct Downloads {
    pub client: FileRef,
}

#[derive(Deserialize)]
pub struct FileRef {
    pub url: String,
    pub sha1: String,
    pub size: u64,
}

// ---------- Tải file ----------

struct Task {
    url: String,
    path: PathBuf,
    sha1: Option<String>,
    size: u64,
}

pub fn make_client() -> Res<Client> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .pool_max_idle_per_host(CONCURRENCY)
        .user_agent("RustLauncher/0.1")
        .build()?)
}

async fn try_download(client: &Client, t: &Task, shared: &Shared, got: &mut u64) -> Res<()> {
    let mut tmp = t.path.clone().into_os_string();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);

    let mut resp = client.get(&t.url).send().await?.error_for_status()?;
    let mut file = fs::File::create(&tmp).await?;
    let mut hasher = Sha1::new();
    while let Some(chunk) = resp.chunk().await? {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        *got += chunk.len() as u64;
        shared.bytes_done.fetch_add(chunk.len() as u64, Relaxed);
    }
    file.flush().await?;
    drop(file);

    if let Some(expect) = &t.sha1 {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expect) {
            return Err(format!("sai SHA-1: {}", t.url).into());
        }
    }
    fs::rename(&tmp, &t.path).await?;
    Ok(())
}

async fn download_one(client: &Client, t: &Task, shared: &Shared) -> Res<()> {
    // Cache: file đã có đúng kích thước thì coi là hợp lệ, không băm lại.
    if t.size > 0 {
        if let Ok(meta) = fs::metadata(&t.path).await {
            if meta.len() == t.size {
                shared.bytes_done.fetch_add(t.size, Relaxed);
                shared.files_done.fetch_add(1, Relaxed);
                return Ok(());
            }
        }
    }
    if let Some(dir) = t.path.parent() {
        fs::create_dir_all(dir).await?;
    }

    let mut last_err: Option<Box<dyn Error + Send + Sync>> = None;
    for _ in 0..3 {
        let mut got = 0u64;
        match try_download(client, t, shared, &mut got).await {
            Ok(()) => {
                shared.files_done.fetch_add(1, Relaxed);
                return Ok(());
            }
            Err(e) => {
                // Trừ phần đã cộng dở để thanh tiến trình không bị lố khi thử lại.
                shared.bytes_done.fetch_sub(got, Relaxed);
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "lỗi tải không rõ".into()))
}

// ---------- API chính ----------

pub async fn load_versions(client: &Client, shared: &Shared) -> Res<()> {
    let manifest: VersionManifest = client
        .get(MANIFEST_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let list: Vec<VersionEntry> = manifest
        .versions
        .into_iter()
        .filter(|v| v.kind == "release")
        .collect();
    *shared.versions.lock().unwrap() = list;
    Ok(())
}

/// Chỉ tải version JSON + client.jar (đủ để đọc texture/model), không tải thư viện hay assets âm thanh.
pub async fn install_client(client: &Client, root: &Path, entry: &VersionEntry, shared: &Arc<Shared>) -> Res<PathBuf> {
    shared.set_phase("Đang đọc thông tin phiên bản...");
    let vjson = root.join("versions").join(&entry.id).join(format!("{}.json", entry.id));
    download_one(
        client,
        &Task {
            url: entry.url.clone(),
            path: vjson.clone(),
            sha1: if entry.sha1.is_empty() { None } else { Some(entry.sha1.clone()) },
            size: 0,
        },
        shared,
    )
    .await?;
    let meta: VersionMeta = serde_json::from_slice(&fs::read(&vjson).await?)?;
    let jar = root.join("versions").join(&entry.id).join(format!("{}.jar", entry.id));
    shared.reset_progress();
    shared.bytes_total.store(meta.downloads.client.size, Relaxed);
    shared.files_total.store(1, Relaxed);
    shared.set_phase("Đang tải client.jar...");
    download_one(
        client,
        &Task {
            url: meta.downloads.client.url.clone(),
            path: jar.clone(),
            sha1: Some(meta.downloads.client.sha1.clone()),
            size: meta.downloads.client.size,
        },
        shared,
    )
    .await?;
    // Ảnh panorama thật nằm ở asset index chứ không nhất thiết trong jar: tải thêm 6 file (best-effort).
    if let Some(ai) = &meta.asset_index {
        match fetch_panorama(client, root, &entry.id, ai, shared).await {
            Ok(n) => shared.notes.lock().unwrap().push(format!("Đã có {n}/6 ảnh panorama từ asset index.")),
            Err(e) => shared.notes.lock().unwrap().push(format!("Không tải được panorama từ asset index: {e}")),
        }
    } else {
        shared.notes.lock().unwrap().push("version JSON không có assetIndex.".into());
    }
    Ok(jar)
}

/// Thư mục chứa panorama_0..5.png tải từ asset index cho một phiên bản.
pub fn pano_dir(root: &Path, version: &str) -> PathBuf {
    root.join("assets").join("panorama").join(version)
}

/// Tải asset index rồi 6 ảnh panorama (kiểm SHA-1, bỏ qua file đã có) vào `pano_dir`.
pub async fn fetch_panorama(client: &Client, root: &Path, version: &str, ai: &AssetIndexRef, shared: &Arc<Shared>) -> Res<usize> {
    shared.set_phase("Đang tải panorama...");
    shared.reset_progress();
    let idx_path = root.join("assets").join("indexes").join(format!("{}.json", ai.id));
    shared.bytes_total.store(ai.size, Relaxed);
    shared.files_total.store(1, Relaxed);
    download_one(client, &Task { url: ai.url.clone(), path: idx_path.clone(), sha1: Some(ai.sha1.clone()), size: ai.size }, shared).await?;
    let objs = crate::mcgui::pano::parse_objects(&fs::read(&idx_path).await?);
    if objs.is_empty() {
        return Err("asset index không có panorama".into());
    }
    shared.reset_progress();
    shared.bytes_total.store(objs.iter().map(|o| o.2).sum(), Relaxed);
    shared.files_total.store(objs.len(), Relaxed);
    let dir = pano_dir(root, version);
    let mut n = 0;
    for (i, hash, size) in objs {
        let url = format!("{OBJECTS_URL}/{}/{}", &hash[..2], hash);
        download_one(client, &Task { url, path: dir.join(format!("panorama_{i}.png")), sha1: Some(hash), size }, shared).await?;
        n += 1;
    }
    Ok(n)
}
