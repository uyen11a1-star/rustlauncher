use eframe::egui;

const VERSIONS: &[&str] = &["1.21.4", "1.20.1", "1.16.5", "1.8.9"];

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
    status: String,
}

impl Default for LauncherApp {
    fn default() -> Self {
        Self {
            username: "Player".to_owned(),
            selected: 0,
            status: "Giai đoạn 1: khung app. Chưa có core tải/chạy game.".to_owned(),
        }
    }
}

impl eframe::App for LauncherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Chừa chỗ cho thanh trạng thái của Android.
            ui.add_space(36.0);
            ui.heading("Rust Launcher");
            ui.separator();

            ui.label("Tên người chơi");
            ui.text_edit_singleline(&mut self.username);

            ui.add_space(8.0);
            ui.label("Phiên bản");
            egui::ComboBox::from_id_salt("version")
                .selected_text(VERSIONS[self.selected])
                .show_ui(ui, |ui| {
                    for (i, v) in VERSIONS.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, i, *v);
                    }
                });

            ui.add_space(16.0);
            if ui.button("Chơi").clicked() {
                self.status = format!(
                    "Sẽ chạy {} với tên {} (chưa cài đặt core).",
                    VERSIONS[self.selected], self.username
                );
                log::info!("{}", self.status);
            }

            ui.add_space(16.0);
            ui.label(&self.status);
        });
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

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
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            // Chữ to hơn cho màn hình cảm ứng.
            cc.egui_ctx.set_pixels_per_point(2.0);
            Ok(Box::new(LauncherApp::default()))
        }),
    )
    .expect("không khởi động được eframe");
}
