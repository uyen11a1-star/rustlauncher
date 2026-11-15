use std::{
    path::PathBuf,
    sync::{atomic::Ordering::Relaxed, Arc},
    time::Duration,
};

use eframe::egui;

mod mc;

use mc::Shared;

/// Font mặc định của egui thiếu một số dấu tiếng Việt, nên nạp font hệ thống Android.
fn setup_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        "/system/fonts/Roboto-Regular.ttf",
        "/system/fonts/RobotoStatic-Regular.ttf",
        "/system/fonts/NotoSans-Regular.ttf",
        "/system/fonts/DroidSans.ttf",
    ];
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("system".to_owned(), egui::FontData::from_owned(bytes));
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "system".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("system".to_owned());
            ctx.set_fonts(fonts);
            log::info!("đã nạp font {path}");
            return;
        }
    }
    log::warn!("không tìm thấy font hệ thống, dùng font mặc định");
}

struct LauncherApp {
    username: String,
    selected: usize,
    shared: Arc<Shared>,
    root: PathBuf,
    rt: tokio::runtime::Runtime,
    client: reqwest::Client,
}

impl LauncherApp {
    fn new(root: PathBuf) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("không tạo được tokio runtime");
        let client = mc::make_client().expect("không tạo được HTTP client");
        let shared = Arc::new(Shared::default());

        // Tải danh sách phiên bản ngay khi mở app.
        {
            let client = client.clone();
            let shared = shared.clone();
            rt.spawn(async move {
                if let Err(e) = mc::load_versions(&client, &shared).await {
                    *shared.error.lock().unwrap() =
                        Some(format!("Không tải được danh sách phiên bản: {e}"));
                }
            });
        }

        Self {
            username: "Player".to_owned(),
            selected: 0,
            shared,
            root,
            rt,
            client,
        }
    }

    fn start_install(&self, entry: mc::VersionEntry) {
        let shared = self.shared.clone();
        let client = self.client.clone();
        let root = self.root.clone();

        shared.running.store(true, Relaxed);
        *shared.error.lock().unwrap() = None;
        shared.reset_progress();

        self.rt.spawn(async move {
            if let Err(e) = mc::install(&client, &root, &entry, &shared).await {
                *shared.error.lock().unwrap() = Some(format!("Lỗi: {e}"));
                shared.set_phase("Thất bại");
            }
            shared.running.store(false, Relaxed);
        });
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let versions = self.shared.versions.lock().unwrap().clone();
        let running = self.shared.running.load(Relaxed);
        let error = self.shared.error.lock().unwrap().clone();
        let phase = self.shared.phase.lock().unwrap().clone();

        // Cập nhật giao diện ~10 lần/giây khi đang tải hoặc đang chờ danh sách.
        if running || (versions.is_empty() && error.is_none()) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // Chừa chỗ cho thanh trạng thái của Android.
            ui.add_space(36.0);
            ui.heading("Rust Launcher");
            ui.separator();

            ui.label("Tên người chơi");
            ui.text_edit_singleline(&mut self.username);

            ui.add_space(8.0);
            ui.label("Phiên bản");
            if versions.is_empty() {
                ui.label("Đang tải danh sách...");
            } else {
                if self.selected >= versions.len() {
                    self.selected = 0;
                }
                egui::ComboBox::from_id_salt("version")
                    .selected_text(versions[self.selected].id.as_str())
                    .show_ui(ui, |ui| {
                        for (i, v) in versions.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, i, v.id.as_str());
                        }
                    });
            }

            ui.add_space(16.0);
            let can_install = !versions.is_empty() && !running;
            if ui
                .add_enabled(can_install, egui::Button::new("Tải phiên bản"))
                .clicked()
            {
                self.start_install(versions[self.selected].clone());
            }

            let total = self.shared.bytes_total.load(Relaxed);
            let done = self.shared.bytes_done.load(Relaxed);
            let files_done = self.shared.files_done.load(Relaxed);
            let files_total = self.shared.files_total.load(Relaxed);

            ui.add_space(16.0);
            if !phase.is_empty() {
                ui.label(phase.as_str());
            }
            if total > 0 {
                let frac = (done as f32 / total as f32).min(1.0);
                ui.add(egui::ProgressBar::new(frac).show_percentage());
                ui.label(format!(
                    "{} / {} MB • {} / {} tệp",
                    done / 1_048_576,
                    total / 1_048_576,
                    files_done,
                    files_total
                ));
            }
            if let Some(e) = &error {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::LIGHT_RED, e.as_str());
            }
        });
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

    // Thư mục dữ liệu riêng của app (không cần xin quyền lưu trữ).
    let root = app
        .external_data_path()
        .or_else(|| app.internal_data_path())
        .unwrap_or_else(|| PathBuf::from("/data/local/tmp"))
        .join("minecraft");
    log::info!("thư mục dữ liệu: {}", root.display());

    // eframe 0.29 chưa có NativeOptions::android_app, nên truyền AndroidApp
    // vào winit qua hook event_loop_builder.
    let options = eframe::NativeOptions {
        event_loop_builder: Some(Box::new(move |builder| {
            use winit::platform::android::EventLoopBuilderExtAndroid;
            builder.with_android_app(app);
        })),
        ..Default::default()
    };

    eframe::run_native(
        "Rust Launcher",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            // Chữ to hơn cho màn hình cảm ứng.
            cc.egui_ctx.set_pixels_per_point(2.0);
            Ok(Box::new(LauncherApp::new(root)))
        }),
    )
    .expect("không khởi động được eframe");
}
