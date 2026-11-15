use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use winit::{
    application::ApplicationHandler,
    event::{TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

mod game;
mod gpu;
mod ui;
mod world;

use game::{Game, Phase};
use world::World;

/// Bán kính nạp chunk, tính theo chunk (1 chunk = 16 khối).
const RENDER_RADIUS: i32 = 6;
/// Mỗi khung hình chỉ dành chừng này thời gian để sinh và dựng chunk mới (tối thiểu một chunk).
const LOAD_BUDGET: Duration = Duration::from_millis(6);

/// Các ô chunk quanh người chơi, xếp từ gần ra xa, để nạp chunk gần trước.
fn chunk_offsets() -> Vec<(i32, i32)> {
    let r = RENDER_RADIUS;
    let max_d2 = ((r as f32 + 0.5) * (r as f32 + 0.5)) as i32;
    let mut v = Vec::new();
    for dx in -(r + 1)..=(r + 1) {
        for dz in -(r + 1)..=(r + 1) {
            if dx * dx + dz * dz <= max_d2 {
                v.push((dx, dz));
            }
        }
    }
    v.sort_by_key(|&(dx, dz)| dx * dx + dz * dz);
    v
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<gpu::Gpu>,
    game: Game,
    world: World,
    /// Các chunk đang có mesh trên GPU.
    loaded: HashSet<(i32, i32)>,
    /// Các chunk vừa bị sửa, cần dựng lại mesh.
    dirty: Vec<(i32, i32)>,
    offsets: Vec<(i32, i32)>,
    last: Instant,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            gpu: None,
            game: Game::new(),
            world: World::new(),
            loaded: HashSet::new(),
            dirty: Vec::new(),
            offsets: chunk_offsets(),
            last: Instant::now(),
        }
    }

    /// Dựng lại chunk bị sửa, bỏ chunk quá xa, nạp thêm chunk gần (trong giới hạn thời gian).
    fn stream_chunks(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let pc = (
            (self.game.pos[0].floor() as i32).div_euclid(world::CS),
            (self.game.pos[2].floor() as i32).div_euclid(world::CS),
        );

        // 1. Chunk vừa sửa: dựng lại ngay (kể cả chunk kề ở rìa).
        for key in self.dirty.drain(..) {
            if self.loaded.contains(&key) {
                self.world.ensure_chunk(key.0, key.1);
                let mesh = self.world.mesh_chunk(key.0, key.1);
                gpu.set_chunk(key, Some(&mesh));
            }
        }

        // 2. Bỏ mesh chunk quá xa (dữ liệu khối vẫn giữ để không mất phần đã sửa).
        let r = RENDER_RADIUS as f32 + 1.5;
        let unload_d2 = (r * r) as i32;
        let far: Vec<(i32, i32)> = self
            .loaded
            .iter()
            .copied()
            .filter(|k| {
                let (dx, dz) = (k.0 - pc.0, k.1 - pc.1);
                dx * dx + dz * dz > unload_d2
            })
            .collect();
        for k in far {
            gpu.set_chunk(k, None);
            self.loaded.remove(&k);
        }

        // 3. Nạp chunk còn thiếu, gần trước xa sau.
        let t0 = Instant::now();
        for &(dx, dz) in &self.offsets {
            let key = (pc.0 + dx, pc.1 + dz);
            if self.loaded.contains(&key) {
                continue;
            }
            self.world.ensure_chunk(key.0, key.1);
            let mesh = self.world.mesh_chunk(key.0, key.1);
            gpu.set_chunk(key, Some(&mesh));
            self.loaded.insert(key);
            if t0.elapsed() > LOAD_BUDGET {
                break;
            }
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
        match gpu::Gpu::new(window.clone()) {
            Ok(g) => {
                self.gpu = Some(g);
                self.window = Some(window);
                // GPU mới chưa có mesh nào: nạp lại từ đầu.
                self.loaded.clear();
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
        self.loaded.clear();
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

                let edited = self.game.update(dt, &mut self.world);
                self.dirty.extend(edited);
                self.stream_chunks();

                if let Some(g) = &mut self.gpu {
                    let eye = self.game.eye();
                    let vp = self.game.view_proj(g.aspect());
                    // Sương mù phải dày đặc trước khi tới rìa vùng đã nạp.
                    let fog_end = (RENDER_RADIUS - 1) as f32 * world::CS as f32;
                    let scene = gpu::SceneUniform::new(vp, eye, fog_end * 0.5, fog_end);
                    let selection = self
                        .game
                        .target()
                        .map(|h| world::outline_mesh(h.block[0], h.block[1], h.block[2]));
                    let ui = ui::build(
                        self.game.layout(),
                        self.game.selected,
                        self.game.flying,
                        self.game.jump_held(),
                        self.game.stick_visual(),
                    );
                    g.render(&scene, selection.as_ref(), &ui);
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
