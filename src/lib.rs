use eframe::egui;

const VERSIONS: &[&str] = &["1.21.4", "1.20.1", "1.16.5", "1.8.9"];

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
        // Chữ to hơn cho màn hình cảm ứng.
        ctx.set_pixels_per_point(2.0);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(16.0);
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
        Box::new(|_cc| Ok(Box::new(LauncherApp::default()))),
    )
    .expect("không khởi động được eframe");
}
