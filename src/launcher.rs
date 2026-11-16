//! Logic nền (không dính giao diện): danh sách phiên bản, tải client.jar, dựng cảnh. Chạy trên luồng riêng.
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, Mutex,
    },
};

use crate::{mc, scene};

static CRASH_LOG: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Ghi từng dòng nhật ký xuống file ngay, để nếu app bị thoát đột ngột vẫn còn dấu vết.
fn breadcrumb(s: &str) {
    use std::io::Write;
    if let Some(p) = CRASH_LOG.get() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{s}");
        }
    }
}

pub struct Launcher {
    pub root: PathBuf,
    pub shared: Arc<mc::Shared>,
    pub log: Arc<Mutex<Vec<String>>>,
    pub busy: Arc<AtomicBool>,
    /// Cảnh 3D đã dựng xong (CPU), chờ luồng chính đưa lên GPU.
    pub scene: Arc<Mutex<Option<scene::SceneData>>>,
}

pub fn pusher(log: &Arc<Mutex<Vec<String>>>) -> impl Fn(String) + Send + Sync + Clone + 'static {
    let log = log.clone();
    move |s: String| {
        breadcrumb(&s);
        log.lock().unwrap().push(s)
    }
}

impl Launcher {
    pub fn new(root: PathBuf) -> Self {
        // Nhật ký bền: đọc phần còn lại của lần chạy trước (nếu app từng bị văng) rồi bắt đầu file mới.
        let crash = root.join("last_run.log");
        let prev = std::fs::read_to_string(&crash).unwrap_or_default();
        let _ = std::fs::write(&crash, "");
        let _ = CRASH_LOG.set(crash);

        let mut first = vec!["Sẵn sàng.".to_string()];
        if !prev.trim().is_empty() {
            first.push("--- Nhật ký lần chạy trước (app bị thoát giữa chừng) ---".into());
            first.extend(prev.lines().map(|x| x.to_string()));
            first.push("--- hết nhật ký cũ ---".into());
        }
        let l = Self {
            root,
            shared: Arc::new(mc::Shared::default()),
            log: Arc::new(Mutex::new(first)),
            busy: Arc::new(AtomicBool::new(false)),
            scene: Arc::new(Mutex::new(None)),
        };
        l.load_versions();
        l
    }

    fn load_versions(&self) {
        let (shared, push) = (self.shared.clone(), pusher(&self.log));
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(r) => r,
                Err(e) => return push(format!("Lỗi tokio: {e}")),
            };
            let r = rt.block_on(async {
                let client = mc::make_client()?;
                mc::load_versions(&client, &shared).await
            });
            match r {
                Ok(()) => push(format!("Đã tải danh sách {} phiên bản.", shared.versions.lock().unwrap().len())),
                Err(e) => push(format!("Không tải được danh sách phiên bản: {e}")),
            }
        });
    }

    /// Tải client.jar rồi dựng cảnh 3D từ texture/model thật. Kết quả để vào `self.scene`.
    pub fn prepare_view(&self, id: String) {
        if self.busy.swap(true, Relaxed) {
            return;
        }
        let (root, shared, log, busy, out) = (self.root.clone(), self.shared.clone(), self.log.clone(), self.busy.clone(), self.scene.clone());
        std::thread::spawn(move || {
            let push = pusher(&log);
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(r) => r,
                Err(e) => {
                    push(format!("tokio: {e}"));
                    busy.store(false, Relaxed);
                    return;
                }
            };
            let res: mc::Res<std::path::PathBuf> = rt.block_on(async {
                let client = mc::make_client()?;
                let entry = shared.versions.lock().unwrap().iter().find(|v| v.id == id).cloned().ok_or("không thấy phiên bản trong danh sách")?;
                let jar = mc::install_client(&client, &root, &entry, &shared).await?;
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(jar)
            });
            match res {
                Err(e) => {
                    push(format!("LỖI: {e}"));
                    shared.set_phase("Lỗi");
                }
                Ok(jar) => {
                    shared.set_phase("Đang dựng texture và model...");
                    match scene::build(&jar, &push) {
                        Ok(d) => {
                            *out.lock().unwrap() = Some(d);
                            shared.set_phase("Sẵn sàng");
                        }
                        Err(e) => {
                            push(format!("LỖI dựng cảnh: {e}"));
                            shared.set_phase("Lỗi");
                        }
                    }
                }
            }
            busy.store(false, Relaxed);
        });
    }
}
