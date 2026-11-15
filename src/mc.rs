//! Core tải Minecraft: manifest, libraries, assets. Tải song song, bỏ qua file đã có.
#![allow(dead_code)]

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed},
        Mutex,
    },
    time::Duration,
};

use futures_util::{stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use tokio::{fs, io::AsyncWriteExt};

pub type Res<T> = Result<T, Box<dyn Error + Send + Sync>>;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const ASSET_BASE: &str = "https://resources.download.minecraft.net";
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
    pub id: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(rename = "assetIndex")]
    pub asset_index: AssetIndexRef,
    pub downloads: Downloads,
    #[serde(default)]
    pub libraries: Vec<Library>,
}

#[derive(Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
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

#[derive(Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibDownloads>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Deserialize)]
pub struct LibDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
}

#[derive(Deserialize)]
pub struct Artifact {
    pub path: String,
    pub url: String,
    pub sha1: String,
    pub size: u64,
}

#[derive(Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
}

#[derive(Deserialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

/// Android chạy nhân Linux nên coi như os = "linux".
/// Lưu ý: các thư viện native LWJGL của desktop sẽ được thay ở giai đoạn 3.
fn lib_allowed(rules: &[Rule]) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut ok = false;
    for r in rules {
        let applies = match &r.os {
            None => true,
            Some(os) => os.name.as_deref().map_or(true, |n| n == "linux"),
        };
        if applies {
            ok = r.action == "allow";
        }
    }
    ok
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

pub async fn install(client: &Client, root: &Path, entry: &VersionEntry, shared: &Shared) -> Res<()> {
    shared.set_phase("Đang đọc thông tin phiên bản...");
    let vjson = root
        .join("versions")
        .join(&entry.id)
        .join(format!("{}.json", entry.id));
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

    let idx_path = root
        .join("assets")
        .join("indexes")
        .join(format!("{}.json", meta.asset_index.id));
    download_one(
        client,
        &Task {
            url: meta.asset_index.url.clone(),
            path: idx_path.clone(),
            sha1: Some(meta.asset_index.sha1.clone()),
            size: meta.asset_index.size,
        },
        shared,
    )
    .await?;
    let index: AssetIndex = serde_json::from_slice(&fs::read(&idx_path).await?)?;

    // Gom toàn bộ file cần tải, loại trùng theo đường dẫn
    // (nhiều tên asset có thể cùng một hash).
    let mut tasks: Vec<Task> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut push = |t: Task| {
        if seen.insert(t.path.clone()) {
            tasks.push(t);
        }
    };

    push(Task {
        url: meta.downloads.client.url.clone(),
        path: root.join("versions").join(&entry.id).join(format!("{}.jar", entry.id)),
        sha1: Some(meta.downloads.client.sha1.clone()),
        size: meta.downloads.client.size,
    });

    for lib in &meta.libraries {
        if !lib_allowed(&lib.rules) {
            continue;
        }
        if let Some(a) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            push(Task {
                url: a.url.clone(),
                path: root.join("libraries").join(&a.path),
                sha1: Some(a.sha1.clone()),
                size: a.size,
            });
        }
    }

    for obj in index.objects.values() {
        let prefix = &obj.hash[..2];
        push(Task {
            url: format!("{ASSET_BASE}/{prefix}/{}", obj.hash),
            path: root.join("assets").join("objects").join(prefix).join(&obj.hash),
            sha1: Some(obj.hash.clone()),
            size: obj.size,
        });
    }

    shared.reset_progress();
    shared.files_total.store(tasks.len(), Relaxed);
    shared
        .bytes_total
        .store(tasks.iter().map(|t| t.size).sum(), Relaxed);

    shared.set_phase("Đang tải thư viện và tài nguyên...");
    let mut results = stream::iter(tasks.iter())
        .map(|t| download_one(client, t, shared))
        .buffer_unordered(CONCURRENCY);
    while let Some(r) = results.next().await {
        r?;
    }

    shared.set_phase("Hoàn tất");
    Ok(())
}
