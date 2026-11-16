//! Rust Launcher: launcher Minecraft Java cho Android, shell tự viết (android-activity + EGL + GLES, không eframe).
use std::{ffi::c_void, path::PathBuf, time::Duration};

use android_activity::{
    input::{InputEvent, MotionAction},
    AndroidApp, InputStatus, MainEvent, PollEvent,
};

mod components;
mod gfx;
mod gles;
mod inflate;
mod jre;
mod json;
mod jvm;
mod launch;
mod launcher;
mod mc;
mod screen;
mod tarx;
mod zipr;

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    let root = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let launcher = launcher::Launcher::new(root);
    let mut screen = screen::Screen::new();

    let mut egl: Option<gles::Egl> = None;
    let mut gfx: Option<gfx::Gfx> = None;
    let mut quit = false;

    while !quit {
        let (mut init, mut term) = (false, false);
        app.poll_events(Some(Duration::from_millis(33)), |event| {
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

        let scale = gfx.as_ref().map_or(1.0, |g| g.scale);
        if let Ok(mut it) = app.input_events_iter() {
            while it.next(|ev| {
                if let InputEvent::MotionEvent(m) = ev {
                    if m.pointer_count() > 0 {
                        let p = m.pointer_at_index(0);
                        let (x, y) = (p.x(), p.y());
                        match m.action() {
                            MotionAction::Down => screen.touch_down(x, y, scale),
                            MotionAction::Move => screen.touch_move(x, y, scale),
                            MotionAction::Up | MotionAction::Cancel => screen.touch_up(x, y),
                            _ => {}
                        }
                    }
                    return InputStatus::Handled;
                }
                InputStatus::Unhandled
            }) {}
        }

        if let (Some(e), Some(g)) = (egl.as_ref(), gfx.as_mut()) {
            if !e.surface.is_null() {
                let (w, h) = e.size();
                g.set_size(w, h);
                if let Some(id) = screen.draw(g, &launcher) {
                    launcher.start(id);
                }
                e.swap();
            }
        }
    }
}
