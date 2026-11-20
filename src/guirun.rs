//! Luồng chạy giao diện game kiểu Minecraft: màn Mojang -> Title -> chọn world -> Loading terrain -> game -> Game Menu.
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering::Relaxed},
        Arc, Mutex,
    },
    time::Instant,
};

use crate::{
    assets::Jar,
    gfx::Gfx,
    glgui::GlCanvas,
    mcgui::{
        assets::{GuiAssets, Source},
        canvas::Draw,
        gui::{Ev, Gui, Screen},
        menu::{Boot, Ptr},
        worlds,
    },
    scene::{self, SceneData},
    view::{self, Phase, View},
};

impl Source for Jar {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        Jar::read(self, path).ok()
    }
}

pub enum Out {
    Stay,
    /// quay về launcher
    Exit,
}

#[derive(PartialEq, Clone, Copy)]
enum Ph {
    /// đang nạp tài nguyên/Title
    Gui,
    /// đang dựng thế giới ở luồng nền
    Loading,
    Game,
    Paused,
}

struct Pt {
    id: Option<i32>,
    start: (f32, f32),
    last: (f32, f32),
    dragged: bool,
}

type BuildSlot = Arc<Mutex<Option<Result<SceneData, String>>>>;

pub struct GuiRun {
    jar_path: PathBuf,
    jar: Arc<Jar>,
    assets: GuiAssets,
    canvas: GlCanvas,
    slot: Arc<Mutex<Option<GuiAssets>>>,
    prog: Arc<(AtomicUsize, AtomicUsize)>,
    loaded: bool,
    boot: Boot,
    gui: Option<Gui>,
    ph: Ph,
    pt: Pt,
    last: Instant,
    build: BuildSlot,
    view: Option<Box<View>>,
    log: Arc<Mutex<Vec<String>>>,
    version: String,
    tz: i64,
    root: PathBuf,
    net_mode: bool,
}

#[cfg(target_os = "android")]
fn tz_minutes() -> i64 {
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: *const u8,
    }
    extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut tm = Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
    unsafe {
        if localtime_r(&now, &mut tm).is_null() {
            return 0;
        }
    }
    tm.gmtoff / 60
}
#[cfg(not(target_os = "android"))]
fn tz_minutes() -> i64 {
    0
}

impl GuiRun {
    /// Cần context GL hiện hành. Mở jar, vẽ được màn Mojang ngay, nạp phần còn lại ở luồng nền.
    pub fn new(jar_path: PathBuf, version: String, log: Arc<Mutex<Vec<String>>>, root: PathBuf) -> Result<GuiRun, String> {
        let jar = Arc::new(Jar::open(&jar_path)?);
        let assets = GuiAssets::mini(&*jar, &version);
        if assets.mojang.is_none() {
            return Err("client.jar không có logo Mojang".into());
        }
        let canvas = GlCanvas::new(&assets.imgs, None)?;
        let slot: Arc<Mutex<Option<GuiAssets>>> = Arc::new(Mutex::new(None));
        let prog = Arc::new((AtomicUsize::new(0), AtomicUsize::new(1)));
        {
            let (jar, slot, prog, ver) = (jar.clone(), slot.clone(), prog.clone(), version.clone());
            std::thread::spawn(move || {
                let a = GuiAssets::load(&*jar, &ver, &|d, t| {
                    prog.0.store(d, Relaxed);
                    prog.1.store(t.max(1), Relaxed);
                });
                *slot.lock().unwrap() = Some(a);
            });
        }
        Ok(GuiRun {
            jar_path,
            jar,
            assets,
            canvas,
            slot,
            prog,
            loaded: false,
            boot: Boot::new(),
            gui: None,
            ph: Ph::Gui,
            pt: Pt { id: None, start: (0.0, 0.0), last: (0.0, 0.0), dragged: false },
            last: Instant::now(),
            build: Arc::new(Mutex::new(None)),
            view: None,
            log,
            version,
            tz: tz_minutes(),
            root,
            net_mode: false,
        })
    }

    fn scale(&self, w: i32, h: i32) -> f32 {
        let auto = ((w as f32 / 320.0).floor().min((h as f32 / 240.0).floor())).max(1.0);
        let s = self.gui.as_ref().map_or(0.0, |g| g.get("guiscale")) as usize;
        if s == 0 {
            auto
        } else {
            (s as f32).min(auto)
        }
    }

    fn start_world(&mut self, i: usize) {
        let Some(g) = self.gui.as_mut() else { return };
        let Some(w) = g.worlds.get(i).cloned() else { return };
        let radius = ((g.get("render") as i32).clamp(4, 24) / 2).max(2);
        g.open(Screen::Loading);
        g.load_msg.clear();
        self.ph = Ph::Loading;
        *self.build.lock().unwrap() = None;
        let (jar, dir, slot, log) = (self.jar_path.clone(), w.dir.clone(), self.build.clone(), self.log.clone());
        std::thread::spawn(move || {
            let push = crate::launcher::pusher(&log);
            push(format!("Vào world: {}", dir.display()));
            let r = scene::build_world(&jar, &dir, radius, &push);
            *slot.lock().unwrap() = Some(r);
        });
    }

    fn start_net(&mut self, conn: crate::mcgui::net::Conn) {
        self.ph = Ph::Loading;
        self.net_mode = true;
        if let Some(gui) = self.gui.as_mut() {
            gui.online = true;
        }
        *self.build.lock().unwrap() = None;
        let (jar, slot, log) = (self.jar_path.clone(), self.build.clone(), self.log.clone());
        std::thread::spawn(move || {
            let push = crate::launcher::pusher(&log);
            let r = crate::netplay::start(conn).and_then(|link| scene::build_net(&jar, link, &push));
            *slot.lock().unwrap() = Some(r);
        });
    }

    pub fn frame(&mut self, g: &mut Gfx, w: i32, h: i32, events: &[(i32, Phase, f32, f32)]) -> Out {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;

        // tài nguyên GUI nạp xong -> đưa lên GPU, dựng Gui
        if !self.loaded {
            let done = self.slot.lock().unwrap().take();
            if let Some(a) = done {
                match GlCanvas::new(&a.imgs, a.pano) {
                    Ok(c) => {
                        self.canvas = c;
                        let mut gui = Gui::new(&a.splashes, &self.version, now.elapsed().as_nanos() as u64 ^ self.tz as u64 ^ std::process::id() as u64);
                        gui.tz_min = self.tz;
                        if let Some(acc) = crate::account::Accounts::load(&self.root).current() {
                            gui.player_name = acc.name.clone();
                        }
                        gui.load_servers(self.root.join("servers.txt"));
                        self.gui = Some(gui);
                        self.log.lock().unwrap().push(format!("Giao diện: {} ảnh, {} sprite, panorama {}", a.imgs.len(), a.sprites.len(), if a.pano.is_some() { "có" } else { "KHÔNG nạp được" }));
                        self.assets = a;
                        self.loaded = true;
                    }
                    Err(e) => {
                        self.log.lock().unwrap().push(format!("LỖI GL giao diện: {e}"));
                        return Out::Exit;
                    }
                }
            }
        }
        let real = if self.loaded { 1.0 } else { self.prog.0.load(Relaxed) as f32 / self.prog.1.load(Relaxed) as f32 };
        self.boot.update(dt, real, self.loaded);

        // thế giới dựng xong
        if self.ph == Ph::Loading {
            let r = self.build.lock().unwrap().take();
            if let Some(gui) = self.gui.as_mut() {
                if let Some(l) = self.log.lock().unwrap().last() {
                    gui.load_msg = l.clone();
                }
            }
            match r {
                Some(Ok(d)) => match View::new(d) {
                    Ok(mut v) => {
                        if let Some(gui) = self.gui.as_ref() {
                            v.fov_deg = gui.get("fov");
                        }
                        self.view = Some(Box::new(v));
                        self.ph = Ph::Game;
                        if let Some(gui) = self.gui.as_mut() {
                            gui.in_game = true;
                        }
                    }
                    Err(e) => self.fail(format!("Failed to create view: {e}")),
                },
                Some(Err(e)) => self.fail(e),
                None => {}
            }
        }

        let (wf, hf) = (w as f32, h as f32);
        match self.ph {
            Ph::Game => {
                if let Some(v) = self.view.as_mut() {
                    for (id, ph, x, y) in events {
                        v.touch(*id, *ph, *x, *y, g);
                    }
                    if let Some(m) = v.take_kick() {
                        self.leave_net(Some(m));
                        return Out::Stay;
                    }
                    if v.frame(g) {
                        v.set_frozen(true);
                        if let Some(gui) = self.gui.as_mut() {
                            gui.in_game = true;
                            gui.reset_to(Screen::Pause);
                        }
                        self.ph = Ph::Paused;
                    }
                }
                return Out::Stay;
            }
            Ph::Paused => {
                if let Some(v) = self.view.as_mut() {
                    v.frame(g);
                }
            }
            _ => {}
        }

        // con trỏ (chạm) theo đơn vị GUI
        let k = self.scale(w, h);
        let mut p = Ptr::default();
        let slop = 14.0;
        for (id, ph, x, y) in events {
            let u = (x / k, y / k);
            match ph {
                Phase::Down => {
                    if self.pt.id.is_none() {
                        self.pt = Pt { id: Some(*id), start: (*x, *y), last: u, dragged: false };
                        p.press = Some(u);
                        p.down = Some(u);
                    }
                }
                Phase::Move => {
                    if self.pt.id == Some(*id) {
                        p.dy += u.1 - self.pt.last.1;
                        self.pt.last = u;
                        if (x - self.pt.start.0).hypot(y - self.pt.start.1) > slop {
                            self.pt.dragged = true;
                        }
                        p.down = Some(u);
                    }
                }
                Phase::Up => {
                    if self.pt.id == Some(*id) {
                        if !self.pt.dragged {
                            p.tap = Some(u);
                        }
                        self.pt.id = None;
                        p.down = None;
                    }
                }
            }
        }
        if p.down.is_none() && self.pt.id.is_some() && p.tap.is_none() {
            p.down = Some(self.pt.last);
        }
        // bỏ qua chạm khi màn Mojang còn đậm
        if self.boot.alpha() > 0.6 {
            p = Ptr::default();
        }

        self.canvas.clear = self.ph != Ph::Paused;
        self.canvas.begin(w, h);
        let mut ev = None;
        {
            let mut d = Draw { c: &mut self.canvas, a: &self.assets, k };
            if let Some(gui) = self.gui.as_mut() {
                ev = gui.frame(&mut d, &p, dt);
            }
            if !self.boot.finished() {
                self.boot.draw(&mut d);
            }
        }
        let _ = (wf, hf);
        if self.gui.is_none() {
            // chưa nạp xong GUI: nền đỏ + logo do Boot vẽ (nền gồm trong Boot::draw)
        }
        self.canvas.end();

        // kết nối server đã đăng nhập xong -> bắt đầu phiên chơi
        if let Some(c) = self.gui.as_mut().and_then(|g| g.joined.take()) {
            self.start_net(c);
        }
        // tiếp nhận sự kiện từ Gui
        let mut out = Out::Stay;
        if let Some(e) = ev {
            match e {
                Ev::Quit => out = Out::Exit,
                Ev::Play(i) => self.start_world(i),
                Ev::Resume => {
                    if let Some(v) = self.view.as_mut() {
                        v.set_frozen(false);
                    }
                    self.ph = Ph::Game;
                }
                Ev::QuitToTitle if self.net_mode => self.leave_net(None),
                Ev::QuitToTitle => {
                    self.view = None;
                    self.ph = Ph::Gui;
                    if let Some(gui) = self.gui.as_mut() {
                        gui.in_game = false;
                        gui.reset_to(Screen::Title);
                        gui.title.t = 0.0;
                        gui.title.fade = 0.0;
                    }
                }
                Ev::Settings => {
                    if let (Some(v), Some(gui)) = (self.view.as_mut(), self.gui.as_ref()) {
                        v.fov_deg = gui.get("fov");
                    }
                }
            }
        }
        let _ = &self.jar;
        out
    }

    /// rời server (bị kick hoặc tự ngắt): về danh sách server, kèm lý do nếu có
    fn leave_net(&mut self, reason: Option<String>) {
        self.view = None;
        self.ph = Ph::Gui;
        self.net_mode = false;
        if let Some(gui) = self.gui.as_mut() {
            gui.in_game = false;
            gui.online = false;
            gui.reset_to(Screen::Title);
            gui.open(Screen::Multi);
            if let Some(m) = reason {
                let m = m.strip_prefix("Disconnected: ").unwrap_or(&m).to_string();
                gui.open(Screen::Disc("Disconnected".into(), m));
            }
        }
    }

    fn fail(&mut self, msg: String) {
        if self.net_mode {
            self.leave_net(Some(msg));
            return;
        }
        self.log.lock().unwrap().push(format!("LỖI vào world: {msg}"));
        self.ph = Ph::Gui;
        if let Some(gui) = self.gui.as_mut() {
            gui.in_game = false;
            gui.reset_to(Screen::Title);
            gui.open(Screen::Msg("Failed to load world".into(), msg));
        }
    }
}

#[allow(dead_code)]
fn _unused(_: &Option<worlds::WorldEntry>, _: &view::Phase) {}
