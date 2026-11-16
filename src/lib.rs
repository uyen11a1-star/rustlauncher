//! Rust Launcher: launcher Minecraft Java cho Android.
//! Mốc 4a: chọn phiên bản, tải, dựng lệnh chạy, gọi `main` thật của Minecraft trong JVM của app.
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc, Mutex,
    },
    time::Duration,
};

use eframe::egui;

mod jre;
mod json;
mod jvm;
mod launch;
mod mc;
mod tarx;

struct App {
    root: PathBuf,
    shared: Arc<mc::Shared>,
    log: Arc<Mutex<Vec<String>>>,
    busy: Arc<AtomicBool>,
    /// JVM chỉ chạy được một lần mỗi tiến trình.
    jvm_used: Arc<AtomicBool>,
    selected: Option<String>,
    show_snapshots: bool,
}

fn setup_fonts(ctx: &egui::Context) {
    // Roboto của hệ thống có đủ dấu tiếng Việt.
    for p in [
        "/system/fonts/Roboto-Regular.ttf",
        "/system/fonts/NotoSans-Regular.ttf",
        "/system/fonts/DroidSans.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(p) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("sys".into(), egui::FontData::from_owned(bytes));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().insert(0, "sys".into());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

fn pusher(log: &Arc<Mutex<Vec<String>>>) -> impl Fn(String) + Send + Sync + Clone + 'static {
    let log = log.clone();
    move |s: String| {
        jvm::breadcrumb(&s);
        log.lock().unwrap().push(s)
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, root: PathBuf) -> Self {
        setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_pixels_per_point(2.0);
        // Nhật ký bền: đọc phần còn lại của lần chạy trước (nếu app từng bị văng) rồi bắt đầu file mới.
        let crash = root.join("last_run.log");
        let prev = std::fs::read_to_string(&crash).unwrap_or_default();
        let _ = std::fs::write(&crash, "");
        jvm::set_crash_log(crash);

        let app = Self {
            root,
            shared: Arc::new(mc::Shared::default()),
            log: Arc::new(Mutex::new({
                let mut v = vec!["Sẵn sàng.".to_string()];
                if !prev.trim().is_empty() {
                    v.push("--- Nhật ký lần chạy trước (app bị thoát giữa chừng) ---".into());
                    v.extend(prev.lines().map(|x| x.to_string()));
                    v.push("--- hết nhật ký cũ ---".into());
                }
                v
            })),
            busy: Arc::new(AtomicBool::new(false)),
            jvm_used: Arc::new(AtomicBool::new(false)),
            selected: None,
            show_snapshots: false,
        };
        app.load_versions();
        app
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

    fn start(&self, id: String) {
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

            // 1. Tải Minecraft + JRE tương ứng
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

            // 2. Chạy JVM trong tiến trình này
            shared.set_phase("Đang chạy Minecraft...");
            used.store(true, Relaxed);
            let _ = std::fs::create_dir_all(&game_dir);
            let _ = std::fs::create_dir_all(root.join("natives"));
            let cp = plan.classpath.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(":");
            let mut args = vec![
                "-Xmx1024m".to_string(),
                "-XX:+UseSerialGC".to_string(),
                format!("-Djava.class.path={cp}"),
            ];
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(200));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(36.0);
            ui.heading("Rust Launcher");
            ui.label("Mốc 4a: chạy main thật của Minecraft (chưa có cầu GLFW)");
            ui.add_space(6.0);

            let busy = self.busy.load(Relaxed);
            let used = self.jvm_used.load(Relaxed);

            ui.checkbox(&mut self.show_snapshots, "Hiện cả snapshot");
            let versions: Vec<String> = self
                .shared
                .versions
                .lock()
                .unwrap()
                .iter()
                .filter(|v| self.show_snapshots || v.kind == "release")
                .map(|v| v.id.clone())
                .collect();
            ui.push_id("versions", |ui| {
                egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                    for id in &versions {
                        let sel = self.selected.as_deref() == Some(id.as_str());
                        if ui.selectable_label(sel, id).clicked() {
                            self.selected = Some(id.clone());
                        }
                    }
                });
            });

            ui.add_space(6.0);
            let can = !busy && !used && self.selected.is_some();
            let label = match (&self.selected, used) {
                (_, true) => "JVM đã chạy (mở lại app để chạy lần nữa)".to_string(),
                (Some(s), _) => format!("Cài và chạy {s}"),
                (None, _) => "Chọn một phiên bản".to_string(),
            };
            if ui.add_enabled(can, egui::Button::new(label)).clicked() {
                if let Some(s) = self.selected.clone() {
                    self.start(s);
                }
            }

            ui.add_space(6.0);
            ui.label(self.shared.phase.lock().unwrap().clone());
            let (d, t) = (self.shared.bytes_done.load(Relaxed), self.shared.bytes_total.load(Relaxed));
            if t > 0 {
                ui.add(egui::ProgressBar::new((d as f32 / t as f32).min(1.0)).text(format!("{} / {} MB", d / 1_000_000, t / 1_000_000)));
            }
            let (fd, ft) = (self.shared.files_done.load(Relaxed), self.shared.files_total.load(Relaxed));
            if ft > 0 {
                ui.label(format!("{fd} / {ft} file"));
            } else if fd > 0 {
                ui.label(format!("{fd} mục đã giải nén"));
            }

            ui.separator();
            ui.push_id("log", |ui| {
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
                    for l in self.log.lock().unwrap().iter() {
                        ui.add(egui::Label::new(egui::RichText::new(l).monospace().size(11.0)).wrap_mode(egui::TextWrapMode::Wrap));
                    }
                });
            });
        });
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    let root = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let opts = eframe::NativeOptions {
        event_loop_builder: Some(Box::new(move |b| {
            b.with_android_app(app);
        })),
        ..Default::default()
    };
    let _ = eframe::run_native("Rust Launcher", opts, Box::new(move |cc| Ok(Box::new(App::new(cc, root)))));
}
