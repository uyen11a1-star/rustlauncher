//! Logic launcher (không dính giao diện): danh sách phiên bản, cài đặt, chạy JVM. Chạy trên luồng riêng.
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, Mutex,
    },
};

use crate::{jre, jvm, launch, mc};

pub struct Launcher {
    pub root: PathBuf,
    pub shared: Arc<mc::Shared>,
    pub log: Arc<Mutex<Vec<String>>>,
    pub busy: Arc<AtomicBool>,
    /// JVM chỉ chạy được một lần mỗi tiến trình.
    pub jvm_used: Arc<AtomicBool>,
}

pub fn pusher(log: &Arc<Mutex<Vec<String>>>) -> impl Fn(String) + Send + Sync + Clone + 'static {
    let log = log.clone();
    move |s: String| {
        jvm::breadcrumb(&s);
        log.lock().unwrap().push(s)
    }
}

impl Launcher {
    pub fn new(root: PathBuf) -> Self {
        // Nhật ký bền: đọc phần còn lại của lần chạy trước (nếu app từng bị văng) rồi bắt đầu file mới.
        let crash = root.join("last_run.log");
        let prev = std::fs::read_to_string(&crash).unwrap_or_default();
        let _ = std::fs::write(&crash, "");
        jvm::set_crash_log(crash);

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
            jvm_used: Arc::new(AtomicBool::new(false)),
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

    /// Cài + chạy một phiên bản. Trả về ngay, việc nặng chạy ở luồng khác.
    pub fn start(&self, id: String) {
        if self.busy.swap(true, Relaxed) {
            return;
        }
        let (root, shared, log, busy, used) = (
            self.root.clone(),
            self.shared.clone(),
            self.log.clone(),
            self.busy.clone(),
            self.jvm_used.clone(),
        );
        // JVM cần stack lớn cho luồng gọi nó.
        let _ = std::thread::Builder::new().stack_size(32 << 20).spawn(move || {
            let push = pusher(&log);
            let fail = |e: String| {
                push(format!("LỖI: {e}"));
                shared.set_phase("Lỗi");
                busy.store(false, Relaxed);
            };
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(r) => r,
                Err(e) => return fail(format!("tokio: {e}")),
            };

            let game_dir = root.join("game");
            let opts = launch::Opts {
                player_name: "Player".into(),
                uuid: "00000000-0000-0000-0000-000000000000".into(),
                access_token: "0".into(),
                user_type: "legacy".into(),
                game_dir: game_dir.clone(),
                assets_root: root.join("assets"),
                natives_dir: root.join("natives"),
                lwjgl_jars: vec![],
            };
            let res: mc::Res<(launch::Plan, PathBuf)> = rt.block_on(async {
                let client = mc::make_client()?;
                let entry = shared
                    .versions
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|v| v.id == id)
                    .cloned()
                    .ok_or("không thấy phiên bản trong danh sách")?;
                push(format!("Cài {id}..."));
                mc::install(&client, &root, &entry, &shared).await?;
                let plan = launch::build_plan(&root, &id, &opts)?;
                push(format!("{id} cần Java {}, main = {}", plan.java_major, plan.main_class));
                let home = jre::install(&root, plan.java_major, &shared, &push).await?;
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>((plan, home))
            });
            let (plan, home) = match res {
                Ok(x) => x,
                Err(e) => return fail(e.to_string()),
            };

            shared.set_phase("Đang chạy Minecraft...");
            used.store(true, Relaxed);
            let _ = std::fs::create_dir_all(&game_dir);
            let _ = std::fs::create_dir_all(root.join("natives"));
            let cp = plan.classpath.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(":");
            let mut args = vec!["-Xmx1024m".to_string(), "-XX:+UseSerialGC".to_string(), format!("-Djava.class.path={cp}")];
            args.extend(plan.jvm_args.iter().cloned());
            let cfg = jvm::JvmConfig {
                lib_dir: jvm::pick_lib_dir(&home, jre::OS_ARCH),
                java_home: home,
                work_dir: game_dir,
                tmp_dir: root.join("tmp"),
                extra_lib_dirs: vec![root.join("natives")],
                args,
                main_class: Some(plan.main_class.clone()),
                app_args: plan.game_args.clone(),
            };
            push(format!("$ java … {} {}", plan.main_class, plan.game_args.join(" ")));
            let cb = {
                let p = push.clone();
                Arc::new(move |s: String| p(format!("[jvm] {s}"))) as Arc<dyn Fn(String) + Send + Sync>
            };
            match jvm::run(&cfg, cb) {
                Ok(code) => {
                    push(format!("main trả về, mã {code}"));
                    shared.set_phase(if code == 0 { "Minecraft đã chạy xong main" } else { "Minecraft báo lỗi (xem nhật ký)" });
                }
                Err(e) => fail(e),
            }
            busy.store(false, Relaxed);
        });
    }
}
