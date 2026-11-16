//! Rust Launcher: launcher Minecraft Java cho Android. Mốc 3: tải JRE và chạy `java -version` trong tiến trình app.
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
mod jvm;
mod mc;
mod tarx;

const JRE_MAJOR: u32 = 21;

struct App {
    root: PathBuf,
    shared: Arc<mc::Shared>,
    log: Arc<Mutex<Vec<String>>>,
    busy: Arc<AtomicBool>,
    /// JVM chỉ chạy được một lần mỗi tiến trình.
    jvm_used: Arc<AtomicBool>,
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

impl App {
    fn new(cc: &eframe::CreationContext<'_>, root: PathBuf) -> Self {
        setup_fonts(&cc.egui_ctx);
        // Nhật ký bền: đọc phần còn lại của lần chạy trước (nếu app từng bị văng) rồi bắt đầu file mới.
        let crash = root.join("last_run.log");
        let prev = std::fs::read_to_string(&crash).unwrap_or_default();
        let _ = std::fs::write(&crash, "");
        jvm::set_crash_log(crash);
        cc.egui_ctx.set_pixels_per_point(2.0);
        Self {
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
        }
    }

    fn start(&self) {
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
        std::thread::spawn(move || {
            let push = {
                let log = log.clone();
                move |s: String| {
                    jvm::breadcrumb(&s);
                    log.lock().unwrap().push(s)
                }
            };
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(r) => r,
                Err(e) => {
                    push(format!("Lỗi tokio: {e}"));
                    busy.store(false, Relaxed);
                    return;
                }
            };
            let res = rt.block_on(jre::install(&root, JRE_MAJOR, &shared, &push));
            match res {
                Err(e) => {
                    push(format!("LỖI: {e}"));
                    shared.set_phase("Lỗi");
                }
                Ok(home) => {
                    shared.set_phase("Đang chạy java -version...");
                    used.store(true, Relaxed);
                    let cfg = jvm::JvmConfig {
                        lib_dir: jvm::pick_lib_dir(&home, jre::OS_ARCH),
                        java_home: home,
                        work_dir: root.join("work"),
                        tmp_dir: root.join("tmp"),
                        extra_lib_dirs: vec![],
                        args: vec!["-XX:+UseSerialGC".into(), "-version".into()],
                    };
                    push("$ java -version".into());
                    let lib_dbg = cfg.lib_dir.display().to_string();
                    jvm::breadcrumb(&format!("lib_dir={lib_dbg}"));
                    let cb = {
                        let log = log.clone();
                        Arc::new(move |s: String| {
                            jvm::breadcrumb(&format!("[jvm] {s}"));
                            log.lock().unwrap().push(s)
                        })
                            as Arc<dyn Fn(String) + Send + Sync>
                    };
                    match jvm::run(&cfg, cb) {
                        Ok(code) => {
                            push(format!("JVM kết thúc, mã thoát {code}"));
                            shared.set_phase(if code == 0 { "Xong: JVM chạy được" } else { "JVM báo lỗi" });
                        }
                        Err(e) => {
                            push(format!("LỖI: {e}"));
                            shared.set_phase("Lỗi");
                        }
                    }
                }
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
            ui.label(format!("Mốc 3: JRE {JRE_MAJOR} + JVM trong app"));
            ui.add_space(8.0);

            let busy = self.busy.load(Relaxed);
            let used = self.jvm_used.load(Relaxed);
            let label = if used { "JVM đã chạy (mở lại app để chạy lần nữa)" } else { "Tải JRE và chạy java -version" };
            if ui.add_enabled(!busy && !used, egui::Button::new(label)).clicked() {
                self.start();
            }

            ui.add_space(8.0);
            ui.label(self.shared.phase.lock().unwrap().clone());
            let (d, t) = (self.shared.bytes_done.load(Relaxed), self.shared.bytes_total.load(Relaxed));
            if t > 0 {
                ui.add(egui::ProgressBar::new(d as f32 / t as f32).text(format!("{} / {} MB", d / 1_000_000, t / 1_000_000)));
            }
            let f = self.shared.files_done.load(Relaxed);
            if f > 0 && t == 0 {
                ui.label(format!("{f} mục đã giải nén"));
            }

            ui.separator();
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
                for l in self.log.lock().unwrap().iter() {
                    ui.add(egui::Label::new(egui::RichText::new(l).monospace().size(11.0)).wrap_mode(egui::TextWrapMode::Wrap));
                }
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
