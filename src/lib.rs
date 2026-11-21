//! Game voxel viết bằng Rust cho Android, đọc dữ liệu từ Minecraft Java (android-activity + EGL + GLES, không JVM).
use std::{ffi::c_void, path::PathBuf, time::Duration};

use android_activity::{
    input::{InputEvent, KeyAction, KeyMapChar, Keycode, MotionAction},
    AndroidApp, InputStatus, MainEvent, PollEvent,
};

mod assets;
mod blocks;
mod chat;
mod craft;
mod gfx;
mod gles;
mod glgui;
mod guirun;
mod mcgui;
mod netdata;
mod netplay;
mod inflate;
mod inv;
mod items;
mod json;
mod launcher;
mod math;
mod mc;
mod nbt;
mod perm;
mod phys;
mod png;
mod scene;
mod slot;
mod screen;
mod view;
mod world;
mod zipr;
mod account;
mod auth;
mod keyboard;
mod ui;
mod vbiome;
mod vcompare;
mod vdens;
mod vnoise;
mod vrng;

enum Mode {
    Menu,
    View(Box<view::View>),
    Gui(Box<guirun::GuiRun>),
}

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    let root = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let launcher = launcher::Launcher::new(root);
    let mut screen = screen::Screen::new();

    let mut egl: Option<gles::Egl> = None;
    let mut gfx: Option<gfx::Gfx> = None;
    let mut quit = false;
    let mut mode = Mode::Menu;
    let mut soft_on = false;

    while !quit {
        let (mut init, mut term) = (false, false);
        let timeout = if matches!(mode, Mode::View(_) | Mode::Gui(_)) { 1 } else { 33 };
        app.poll_events(Some(Duration::from_millis(timeout)), |event| {
            if let PollEvent::Main(me) = event {
                match me {
                    MainEvent::InitWindow { .. } => init = true,
                    MainEvent::TerminateWindow { .. } => term = true,
                    MainEvent::Destroy => quit = true,
                    _ => {}
                }
            }
        });

        if term {
            if let Some(e) = egl.as_mut() {
                e.detach();
            }
        }
        if init {
            if let Some(win) = app.native_window() {
                let ptr = win.ptr().as_ptr() as *mut c_void;
                if egl.is_none() {
                    match gles::Egl::new() {
                        Ok(e) => egl = Some(e),
                        Err(e) => log::error!("EGL: {e}"),
                    }
                }
                if let Some(e) = egl.as_mut() {
                    match e.attach(ptr) {
                        Ok(_) => {
                            if gfx.is_none() {
                                match gfx::Gfx::new() {
                                    Ok(g) => gfx = Some(g),
                                    Err(e) => log::error!("Gfx: {e}"),
                                }
                            }
                        }
                        Err(e) => log::error!("attach: {e}"),
                    }
                }
            }
        }

        // thu mọi sự kiện cảm ứng, xử lý sau theo chế độ hiện tại
        let mut events: Vec<(i32, view::Phase, f32, f32)> = Vec::new();
        let mut keys: Vec<view::KeyIn> = Vec::new();
        let text_on = matches!(&mode, Mode::Gui(r) if r.wants_text());
        if let Ok(mut it) = app.input_events_iter() {
            while it.next(|ev| {
                if let InputEvent::KeyEvent(k) = ev {
                    if !text_on {
                        return InputStatus::Unhandled;
                    }
                    if matches!(k.action(), KeyAction::Down | KeyAction::Multiple) {
                        match k.key_code() {
                            Keycode::Del => keys.push(view::KeyIn::Back),
                            Keycode::Enter | Keycode::NumpadEnter => keys.push(view::KeyIn::Enter),
                            Keycode::Back | Keycode::Escape => keys.push(view::KeyIn::Esc),
                            code => {
                                if let Ok(map) = app.device_key_character_map(k.device_id()) {
                                    if let Ok(KeyMapChar::Unicode(c)) = map.get(code, k.meta_state()) {
                                        keys.push(view::KeyIn::Char(c));
                                    }
                                }
                            }
                        }
                    }
                    return InputStatus::Handled;
                }
                if let InputEvent::MotionEvent(m) = ev {
                    let n = m.pointer_count();
                    match m.action() {
                        MotionAction::Down | MotionAction::PointerDown | MotionAction::Up | MotionAction::PointerUp => {
                            let p = m.pointer_at_index(m.pointer_index());
                            let ph = if matches!(m.action(), MotionAction::Down | MotionAction::PointerDown) { view::Phase::Down } else { view::Phase::Up };
                            events.push((p.pointer_id(), ph, p.x(), p.y()));
                        }
                        MotionAction::Move => {
                            for i in 0..n {
                                let p = m.pointer_at_index(i);
                                events.push((p.pointer_id(), view::Phase::Move, p.x(), p.y()));
                            }
                        }
                        MotionAction::Cancel => {
                            for i in 0..n {
                                let p = m.pointer_at_index(i);
                                events.push((p.pointer_id(), view::Phase::Up, p.x(), p.y()));
                            }
                        }
                        _ => {}
                    }
                    return InputStatus::Handled;
                }
                InputStatus::Unhandled
            }) {}
        }

        // cảnh 3D vừa dựng xong ở luồng nền -> đưa lên GPU, chuyển chế độ
        if matches!(mode, Mode::Menu) && egl.as_ref().map_or(false, |e| !e.surface.is_null()) {
            let ready = launcher.scene.lock().unwrap().take();
            if let Some(d) = ready {
                match view::View::new(d) {
                    Ok(v) => mode = Mode::View(Box::new(v)),
                    Err(e) => launcher.log.lock().unwrap().push(format!("LỖI tạo view: {e}")),
                }
            }
        }

        // client.jar đã sẵn sàng (bấm PLAY) -> mở giao diện game kiểu Minecraft
        if matches!(mode, Mode::Menu) && egl.as_ref().map_or(false, |e| !e.surface.is_null()) {
            let jar = launcher.jar_slot.lock().unwrap().take();
            if let Some((path, id)) = jar {
                match guirun::GuiRun::new(path, id, launcher.log.clone(), launcher.root.clone()) {
                    Ok(r) => mode = Mode::Gui(Box::new(r)),
                    Err(e) => launcher.log.lock().unwrap().push(format!("LỖI mở giao diện game: {e}")),
                }
            }
        }

        if let (Some(e), Some(g)) = (egl.as_ref(), gfx.as_mut()) {
            if !e.surface.is_null() {
                let (w, h) = e.size();
                g.set_size(w, h);
                match &mut mode {
                    Mode::Menu => {
                        let scale = g.scale;
                        for (_, ph, x, y) in &events {
                            match ph {
                                view::Phase::Down => screen.touch_down(*x, *y, scale),
                                view::Phase::Move => screen.touch_move(*x, *y, scale),
                                view::Phase::Up => screen.touch_up(*x, *y),
                            }
                        }
                        match screen.draw(g, &launcher) {
                            Some(screen::Action::View(id)) => launcher.prepare(id, false),
                            Some(screen::Action::World(id)) => {
                                let vm = app.vm_as_ptr();
                                if perm::has_all_files(vm) {
                                    launcher.fetch_jar(id);
                                } else {
                                    let say = launcher::pusher(&launcher.log);
                                    match perm::request_all_files(vm, app.activity_as_ptr(), "dev.rustlauncher.app") {
                                        Ok(()) => say("Cần quyền quản lý mọi tệp: bật cho Rust Launcher ở trang vừa mở, rồi quay lại và bấm PLAY lần nữa.".into()),
                                        Err(e) => say(format!("Không mở được trang cấp quyền ({e}). Tự vào Cài đặt > Ứng dụng > Rust Launcher > Quyền để bật quản lý mọi tệp.")),
                                    }
                                }
                            }
                            None => {}
                        }
                    }
                    Mode::Gui(r) => {
                        for k in keys.drain(..) {
                            r.key_input(k);
                        }
                        let want = r.wants_text();
                        if want != soft_on {
                            soft_on = want;
                            if want {
                                app.show_soft_input(true);
                            } else {
                                app.hide_soft_input(false);
                            }
                        }
                        if matches!(r.frame(g, w, h, &events), guirun::Out::Exit) {
                            mode = Mode::Menu;
                        }
                    }
                    Mode::View(v) => {
                        for (id, ph, x, y) in &events {
                            v.touch(*id, *ph, *x, *y, g);
                        }
                        if v.frame(g) {
                            mode = Mode::Menu;
                        }
                    }
                }
                e.swap();
            }
        }
    }
}
