use std::{sync::Arc, time::Instant};

use winit::{
    application::ApplicationHandler,
    event::{TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

mod game;
mod gpu;
mod world;

use game::{Game, Phase};

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<gpu::Gpu>,
    game: Game,
    meshes: Vec<world::MeshData>,
    last: Instant,
}

impl App {
    fn new() -> Self {
        let t0 = Instant::now();
        let world = world::World::new(4);
        let meshes: Vec<_> = world
            .chunk_coords()
            .into_iter()
            .map(|(cx, cz)| world.mesh_chunk(cx, cz))
            .collect();
        let verts: usize = meshes.iter().map(|m| m.vertices.len()).sum();
        log::info!(
            "sinh thế giới: {} chunk, {} đỉnh, {:?}",
            meshes.len(),
            verts,
            t0.elapsed()
        );
        Self {
            window: None,
            gpu: None,
            game: Game::new(),
            meshes,
            last: Instant::now(),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let window = match el.create_window(Window::default_attributes().with_title("Rust Voxel")) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("không tạo được cửa sổ: {e}");
                el.exit();
                return;
            }
        };
        let size = window.inner_size();
        self.game.set_screen(size.width as f32, size.height as f32);
        match gpu::Gpu::new(window.clone(), &self.meshes) {
            Ok(g) => {
                self.gpu = Some(g);
                self.window = Some(window);
                self.last = Instant::now();
            }
            Err(e) => {
                log::error!("khởi tạo GPU thất bại: {e}");
                el.exit();
            }
        }
    }

    fn suspended(&mut self, _el: &ActiveEventLoop) {
        // Android huỷ cửa sổ gốc: phải bỏ surface trước.
        self.gpu = None;
        self.window = None;
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let Some(g) = &mut self.gpu {
                    g.resize(s.width, s.height);
                }
                self.game.set_screen(s.width as f32, s.height as f32);
            }
            WindowEvent::Touch(t) => {
                let phase = match t.phase {
                    TouchPhase::Started => Phase::Started,
                    TouchPhase::Moved => Phase::Moved,
                    TouchPhase::Ended | TouchPhase::Cancelled => Phase::Ended,
                };
                self.game
                    .touch(t.id, phase, t.location.x as f32, t.location.y as f32);
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last).as_secs_f32().min(0.1);
                self.last = now;
                self.game.update(dt);
                if let Some(g) = &mut self.gpu {
                    let vp = self.game.view_proj(g.aspect());
                    g.render(&vp);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

    let mut builder = EventLoop::builder();
    builder.with_android_app(app);
    let event_loop = builder.build().expect("không tạo được event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut state = App::new();
    event_loop.run_app(&mut state).expect("event loop lỗi");
}
