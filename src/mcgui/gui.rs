//! Bộ máy màn hình: Title, chọn world, tạo world, nhiều người chơi, Options (+ trang con), Game Menu, tải địa hình.
use std::collections::HashMap;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use super::{canvas::*, menu::*, net, worlds::*};

#[derive(Clone, PartialEq)]
pub enum Screen {
    Title,
    Worlds,
    Create,
    Multi,
    Options,
    Page(&'static str),
    Pause,
    Msg(String, String),
    Loading,
    EditServer,
    Direct,
    ConfirmDelete(usize),
    Connecting,
    /// lý do bị ngắt kết nối (tiêu đề, nội dung)
    Disc(String, String),
}

pub struct Server {
    pub name: String,
    pub addr: String,
    pub st: Arc<Mutex<net::Status>>,
}

pub enum Ev {
    /// vào world thứ i
    Play(usize),
    /// Game Menu: về game
    Resume,
    /// Save and Quit to Title
    QuitToTitle,
    /// thoát app
    Quit,
    /// thay đổi cài đặt (FOV, tầm nhìn, GUI scale...)
    Settings,
}

#[derive(Clone, Copy)]
enum Kind {
    /// min, max, bước
    Slider(f32, f32, f32),
    Cycle(&'static [&'static str]),
    Toggle,
}

struct Opt {
    key: &'static str,
    label: &'static str,
    kind: Kind,
    full: bool,
}

const fn o(key: &'static str, label: &'static str, kind: Kind) -> Opt {
    Opt { key, label, kind, full: false }
}
const fn of(key: &'static str, label: &'static str, kind: Kind) -> Opt {
    Opt { key, label, kind, full: true }
}

const PCT: Kind = Kind::Slider(0.0, 1.0, 0.01);
const GFX: &[&str] = &["Fast", "Fancy", "Fabulous!"];
const CLOUDS: &[&str] = &["OFF", "Fast", "Fancy"];
const PARTS: &[&str] = &["All", "Decreased", "Minimal"];
const GUIS: &[&str] = &["Auto", "1", "2", "3", "4", "5", "6"];
const ATTACK: &[&str] = &["Off", "Crosshair", "Hotbar"];
const MIPS: &[&str] = &["OFF", "1", "2", "3", "4"];
const CHATV: &[&str] = &["Shown", "Commands Only", "Hidden"];
const HAND: &[&str] = &["Left", "Right"];
const NARR: &[&str] = &["OFF", "All", "Chat", "System"];

const VIDEO: &[Opt] = &[
    o("graphics", "Graphics", Kind::Cycle(GFX)),
    o("render", "Render Distance", Kind::Slider(2.0, 32.0, 1.0)),
    o("simulation", "Simulation Distance", Kind::Slider(5.0, 32.0, 1.0)),
    o("fps", "Max Framerate", Kind::Slider(10.0, 260.0, 10.0)),
    o("vsync", "VSync", Kind::Toggle),
    o("bobbing", "View Bobbing", Kind::Toggle),
    o("guiscale", "GUI Scale", Kind::Cycle(GUIS)),
    o("attack", "Attack Indicator", Kind::Cycle(ATTACK)),
    o("bright", "Brightness", PCT),
    o("clouds", "Clouds", Kind::Cycle(CLOUDS)),
    o("fullscreen", "Fullscreen", Kind::Toggle),
    o("particles", "Particles", Kind::Cycle(PARTS)),
    o("mipmap", "Mipmap Levels", Kind::Cycle(MIPS)),
    o("shadows", "Entity Shadows", Kind::Toggle),
    o("distort", "Distortion Effects", PCT),
    o("fovfx", "FOV Effects", PCT),
    o("biome", "Biome Blend", Kind::Slider(0.0, 7.0, 1.0)),
    o("entdist", "Entity Distance", Kind::Slider(0.5, 5.0, 0.1)),
];

const SOUND: &[Opt] = &[
    of("vol_master", "Master Volume", PCT),
    o("vol_music", "Music", PCT),
    o("vol_record", "Jukebox/Note Blocks", PCT),
    o("vol_weather", "Weather", PCT),
    o("vol_block", "Blocks", PCT),
    o("vol_hostile", "Hostile Creatures", PCT),
    o("vol_neutral", "Friendly Creatures", PCT),
    o("vol_player", "Players", PCT),
    o("vol_ambient", "Ambient/Environment", PCT),
    o("vol_voice", "Voice/Speech", PCT),
    o("subtitles", "Show Subtitles", Kind::Toggle),
    o("directional", "Directional Audio", Kind::Toggle),
];

const CONTROLS: &[Opt] = &[
    o("sens", "Sensitivity", PCT),
    o("invert", "Invert Mouse", Kind::Toggle),
    o("autojump", "Auto-Jump", Kind::Toggle),
    o("sneak", "Toggle Sneak", Kind::Toggle),
    o("sprint", "Toggle Sprint", Kind::Toggle),
    o("touch", "Touchscreen Mode", Kind::Toggle),
    o("wheel", "Discrete Scrolling", Kind::Toggle),
    o("scrollsens", "Scroll Sensitivity", Kind::Slider(0.1, 10.0, 0.1)),
];

const SKIN: &[Opt] = &[
    o("cape", "Cape: ON", Kind::Toggle),
    o("jacket", "Jacket: ON", Kind::Toggle),
    o("lsleeve", "Left Sleeve: ON", Kind::Toggle),
    o("rsleeve", "Right Sleeve: ON", Kind::Toggle),
    o("lpants", "Left Pants Leg: ON", Kind::Toggle),
    o("rpants", "Right Pants Leg: ON", Kind::Toggle),
    o("hat", "Hat: ON", Kind::Toggle),
    o("hand", "Main Hand", Kind::Cycle(HAND)),
];

const CHAT: &[Opt] = &[
    o("chatvis", "Chat", Kind::Cycle(CHATV)),
    o("colors", "Colors", Kind::Toggle),
    o("links", "Web Links", Kind::Toggle),
    o("chatopacity", "Chat Text Opacity", PCT),
    o("chatscale", "Chat Text Size", PCT),
    o("chatwidth", "Chat Width", PCT),
    o("narrator", "Narrator", Kind::Cycle(NARR)),
    o("autocmd", "Command Suggestions", Kind::Toggle),
];

const ACCESS: &[Opt] = &[
    o("narrator", "Narrator", Kind::Cycle(NARR)),
    o("subtitles", "Show Subtitles", Kind::Toggle),
    o("textbg", "Text Background Opacity", PCT),
    o("contrast", "Menu Background Blurriness", Kind::Slider(0.0, 10.0, 1.0)),
    o("hidesplash", "Hide Splash Texts", Kind::Toggle),
    o("monochrome", "Monochrome Logo", Kind::Toggle),
    o("panorama", "Panorama Scroll Speed", PCT),
    o("darkpulse", "Darkness Pulsing", PCT),
];

const NONE: &[Opt] = &[];

fn page_opts(p: &str) -> &'static [Opt] {
    match p {
        "video" => VIDEO,
        "sound" => SOUND,
        "controls" => CONTROLS,
        "skin" => SKIN,
        "chat" => CHAT,
        "access" => ACCESS,
        _ => NONE,
    }
}

fn page_title(p: &str) -> &'static str {
    match p {
        "video" => "Video Settings",
        "sound" => "Music & Sounds",
        "controls" => "Controls",
        "skin" => "Skin Customization",
        "chat" => "Chat Settings",
        "access" => "Accessibility Settings",
        "lang" => "Language",
        "packs" => "Select Resource Packs",
        "telemetry" => "Telemetry Data",
        "credits" => "Credits & Attribution",
        _ => "",
    }
}

fn defaults() -> HashMap<&'static str, f32> {
    let mut m = HashMap::new();
    let all: [&[Opt]; 6] = [VIDEO, SOUND, CONTROLS, SKIN, CHAT, ACCESS];
    for l in all {
        for op in l {
            let v = match op.kind {
                Kind::Slider(mn, mx, _) => match op.key {
                    "render" => 12.0,
                    "simulation" => 12.0,
                    "fps" => 260.0,
                    "bright" => 0.5,
                    "sens" => 0.5,
                    "distort" | "fovfx" => 1.0,
                    "biome" => 2.0,
                    "entdist" => 1.0,
                    "scrollsens" => 1.0,
                    "chatopacity" | "chatwidth" | "chatscale" | "panorama" | "darkpulse" => 1.0,
                    "textbg" => 0.5,
                    "contrast" => 5.0,
                    "vol_music" => 1.0,
                    _ => if mx <= 1.0 && mn == 0.0 { 1.0 } else { mn },
                },
                Kind::Cycle(_) => match op.key {
                    "graphics" => 1.0,
                    "clouds" => 2.0,
                    "attack" => 1.0,
                    "mipmap" => 4.0,
                    "chatvis" => 0.0,
                    "hand" => 1.0,
                    _ => 0.0,
                },
                Kind::Toggle => match op.key {
                    "vsync" | "bobbing" | "shadows" | "autojump" | "colors" | "links" | "autocmd" | "directional" | "cape" | "jacket" | "lsleeve" | "rsleeve" | "lpants" | "rpants" | "hat" => 1.0,
                    _ => 0.0,
                },
            };
            m.insert(op.key, v);
        }
    }
    m.insert("fov", 70.0);
    m
}

fn fmt_val(op: &Opt, v: f32) -> String {
    match op.kind {
        Kind::Toggle => {
            if op.label.ends_with(": ON") {
                let base = &op.label[..op.label.len() - 4];
                format!("{base}: {}", if v > 0.5 { "ON" } else { "OFF" })
            } else {
                format!("{}: {}", op.label, if v > 0.5 { "ON" } else { "OFF" })
            }
        }
        Kind::Cycle(l) => format!("{}: {}", op.label, l[(v as usize).min(l.len() - 1)]),
        Kind::Slider(mn, mx, _) => {
            let s = match op.key {
                "render" | "simulation" => format!("{} chunks", v as i32),
                "fps" => if v >= 260.0 { "Unlimited".into() } else { format!("{} fps", v as i32) },
                "bright" => if v <= 0.0 { "Moody".into() } else if v >= 1.0 { "Bright".into() } else { format!("{}%", (v * 100.0).round() as i32) },
                "sens" => if v <= 0.0 { "*yawn*".into() } else if v >= 1.0 { "HYPERSPEED!!!".into() } else { format!("{}%", (v * 200.0).round() as i32) },
                "biome" => if v <= 0.0 { "OFF".into() } else { format!("{}x{}", 2 * v as i32 + 1, 2 * v as i32 + 1) },
                "entdist" => format!("{}%", (v * 100.0).round() as i32),
                "contrast" => format!("{}", v as i32),
                "scrollsens" => format!("{:.1}", v),
                _ => if mx <= 1.0 && mn == 0.0 {
                    if v <= 0.0 { "OFF".into() } else { format!("{}%", (v * 100.0).round() as i32) }
                } else {
                    format!("{}", v as i32)
                },
            };
            format!("{}: {}", op.label, s)
        }
    }
}

pub struct Gui {
    pub screen: Screen,
    stack: Vec<Screen>,
    pub in_game: bool,
    pub title: Title,
    pub worlds: Vec<WorldEntry>,
    pub worlds_err: Option<String>,
    sel: Option<usize>,
    last_tap: (usize, f32),
    scroll: f32,
    drag: Option<&'static str>,
    pub set: HashMap<&'static str, f32>,
    pub t: f32,
    pub version: String,
    pub tz_min: i64,
    pub load_msg: String,
    pub load_pct: f32,
    cmode: usize,
    cdiff: usize,
    ccheat: bool,
    quit_confirm: bool,
    pub servers: Vec<Server>,
    pub servers_path: Option<PathBuf>,
    pub player_name: String,
    sel_srv: Option<usize>,
    edit_idx: Option<usize>,
    edit_name: String,
    edit_addr: String,
    focus: usize,
    shift: bool,
    conn: Option<Arc<Mutex<Option<net::Login>>>>,
    conn_t: f32,
    /// kết nối đã đăng nhập, chờ GuiRun nhận
    pub joined: Option<net::Conn>,
    /// đang chơi trên server (nút thoát ghi "Disconnect")
    pub online: bool,
}

impl Gui {
    pub fn new(splashes: &[String], version: &str, seed: u64) -> Gui {
        Gui {
            screen: Screen::Title,
            stack: Vec::new(),
            in_game: false,
            title: Title::new(splashes, seed),
            worlds: Vec::new(),
            worlds_err: None,
            sel: None,
            last_tap: (usize::MAX, -10.0),
            scroll: 0.0,
            drag: None,
            set: defaults(),
            t: 0.0,
            version: version.into(),
            tz_min: 0,
            load_msg: String::new(),
            load_pct: 0.0,
            cmode: 0,
            cdiff: 2,
            ccheat: false,
            quit_confirm: false,
            servers: Vec::new(),
            servers_path: None,
            player_name: "Player".into(),
            sel_srv: None,
            edit_idx: None,
            edit_name: String::new(),
            edit_addr: String::new(),
            focus: 0,
            shift: false,
            conn: None,
            conn_t: 0.0,
            joined: None,
            online: false,
        }
    }

    pub fn load_servers(&mut self, path: PathBuf) {
        if let Ok(t) = std::fs::read_to_string(&path) {
            for l in t.lines() {
                if let Some((n, a)) = l.split_once('\t') {
                    self.servers.push(Server { name: n.to_string(), addr: a.to_string(), st: Arc::new(Mutex::new(net::Status::default())) });
                }
            }
        }
        self.servers_path = Some(path);
    }

    fn save_servers(&self) {
        if let Some(p) = &self.servers_path {
            let t: String = self.servers.iter().map(|s| format!("{}\t{}\n", s.name.replace(['\t', '\n'], " "), s.addr.replace(['\t', '\n'], " "))).collect();
            let _ = std::fs::write(p, t);
        }
    }

    /// ping lại mọi server ở luồng nền
    pub fn refresh_servers(&mut self) {
        for s in &self.servers {
            *s.st.lock().unwrap() = net::Status::default();
            let (st, addr) = (s.st.clone(), s.addr.clone());
            std::thread::spawn(move || {
                let r = net::ping(&addr);
                *st.lock().unwrap() = r;
            });
        }
    }

    fn ping_one(&mut self, i: usize) {
        let s = &self.servers[i];
        *s.st.lock().unwrap() = net::Status::default();
        let (st, addr) = (s.st.clone(), s.addr.clone());
        std::thread::spawn(move || {
            let r = net::ping(&addr);
            *st.lock().unwrap() = r;
        });
    }

    fn join(&mut self, addr: String) {
        let slot: Arc<Mutex<Option<net::Login>>> = Arc::new(Mutex::new(None));
        self.conn = Some(slot.clone());
        self.conn_t = 0.0;
        let name = self.player_name.clone();
        std::thread::spawn(move || {
            let r = net::login(&addr, &name);
            *slot.lock().unwrap() = Some(r);
        });
        self.open(Screen::Connecting);
    }

    pub fn get(&self, k: &str) -> f32 {
        self.set.get(k).copied().unwrap_or(0.0)
    }

    pub fn open(&mut self, s: Screen) {
        let cur = std::mem::replace(&mut self.screen, s);
        self.stack.push(cur);
        self.scroll = 0.0;
    }

    pub fn back(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.screen = s;
        }
        self.scroll = 0.0;
    }

    pub fn reset_to(&mut self, s: Screen) {
        self.stack.clear();
        self.screen = s;
        self.scroll = 0.0;
    }

    pub fn set_worlds(&mut self, r: Result<Vec<WorldEntry>, String>) {
        match r {
            Ok(v) => {
                self.worlds = v;
                self.worlds_err = None;
            }
            Err(e) => {
                self.worlds.clear();
                self.worlds_err = Some(e);
            }
        }
        self.sel = None;
    }

    fn background(&mut self, d: &mut Draw, list: bool) {
        let (w, h) = d.size();
        if self.in_game {
            d.gradient(0.0, 0.0, w, h, 0xc0101010, 0xd0101010);
        } else {
            panorama_bg(d, self.t, 5.0);
            d.fill(0.0, 0.0, w, h, 0x00000040);
        }
        let _ = list;
    }

    fn header(&self, d: &mut Draw, title: &str, y: f32) {
        let (w, _) = d.size();
        d.text_centered(title, (w / 2.0).floor(), y, WHITE);
    }

    /// nền danh sách (menu_list_background) kèm vạch ngăn trên/dưới
    fn list_area(&self, d: &mut Draw, top: f32, bottom: f32) {
        let (w, _h) = d.size();
        let (hs, fs) = if self.in_game { (d.a.header_sep, d.a.footer_sep) } else { (d.a.header_sep, d.a.footer_sep) };
        let lb = if self.in_game { d.a.list_bg } else { d.a.list_bg };
        if let Some(t) = lb {
            let (iw, ih) = (d.a.imgs[t].w as f32, d.a.imgs[t].h as f32);
            let mut y = 0.0;
            while y < bottom - top {
                let hh = (bottom - top - y).min(32.0);
                let mut x = 0.0;
                while x < w {
                    let ww = (w - x).min(32.0);
                    d.blit(t, [x, top + y, ww, hh], [0.0, 0.0, iw * ww / 32.0, ih * hh / 32.0], WHITE);
                    x += 32.0;
                }
                y += 32.0;
            }
        }
        if let Some(t) = hs {
            d.blit_logical(t, [0.0, top - 2.0, w, 2.0], [0.0, 0.0, 32.0, 2.0], 32.0, 2.0, WHITE);
        }
        if let Some(t) = fs {
            d.blit_logical(t, [0.0, bottom, w, 2.0], [0.0, 0.0, 32.0, 2.0], 32.0, 2.0, WHITE);
        }
    }

    pub fn frame(&mut self, d: &mut Draw, p: &Ptr, dt: f32) -> Option<Ev> {
        self.t += dt;
        if !(p.down.is_some()) {
            self.drag = None;
        }
        let (w, h) = d.size();
        let cx = (w / 2.0).floor();
        let scr = self.screen.clone();
        match scr {
            Screen::Title => {
                let ver = self.version.clone();
                match self.title.draw(d, p, dt, &ver) {
                    Some(TitleAction::Singleplayer) => {
                        self.open(Screen::Worlds);
                        self.set_worlds(scan("/sdcard/RustVoxel"));
                    }
                    Some(TitleAction::Multiplayer) => {
                        self.open(Screen::Multi);
                        self.refresh_servers();
                    }
                    Some(TitleAction::Realms) => {
                        self.open(Screen::Msg("Minecraft Realms".into(), "Realms is not available in this build.".into()));
                    }
                    Some(TitleAction::Options) => self.open(Screen::Options),
                    Some(TitleAction::Language) => self.open(Screen::Page("lang")),
                    Some(TitleAction::Accessibility) => self.open(Screen::Page("access")),
                    Some(TitleAction::Quit) => return Some(Ev::Quit),
                    None => {}
                }
                None
            }
            Screen::Worlds => self.worlds_screen(d, p, w, h, cx),
            Screen::Create => {
                self.background(d, false);
                self.header(d, "Create New World", 15.0);
                let mut ev = None;
                self.list_area(d, 33.0, h - 33.0);
                let modes = ["Survival", "Hardcore", "Creative"];
                let diffs = ["Peaceful", "Easy", "Normal", "Hard"];
                d.text_centered("World Name", cx, 41.0, GRAY);
                d.sprite("widget/text_field", cx - 100.0, 52.0, 200.0, 20.0);
                d.text("New World", cx - 94.0, 58.0, 0xe0e0e0ff);
                if button(d, p, [cx - 100.0, 82.0, 200.0, 20.0], &format!("Game Mode: {}", modes[self.cmode]), true) {
                    self.cmode = (self.cmode + 1) % 3;
                }
                if button(d, p, [cx - 100.0, 106.0, 200.0, 20.0], &format!("Difficulty: {}", diffs[self.cdiff]), true) {
                    self.cdiff = (self.cdiff + 1) % 4;
                }
                if button(d, p, [cx - 100.0, 130.0, 200.0, 20.0], &format!("Allow Commands: {}", if self.ccheat { "ON" } else { "OFF" }), true) {
                    self.ccheat = !self.ccheat;
                }
                if button(d, p, [cx - 155.0, h - 27.0, 150.0, 20.0], "Create New World", true) {
                    self.open(Screen::Msg("Create New World".into(), "World generation is still under construction in this build. Copy a world into /sdcard/RustVoxel.".into()));
                }
                if button(d, p, [cx + 5.0, h - 27.0, 150.0, 20.0], "Cancel", true) {
                    self.back();
                }
                ev.take()
            }
            Screen::Multi => self.multi_screen(d, p, w, h, cx),
            Screen::EditServer => self.edit_screen(d, p, w, h, cx, false),
            Screen::Direct => self.edit_screen(d, p, w, h, cx, true),
            Screen::ConfirmDelete(i) => {
                self.background(d, false);
                let nm = self.servers.get(i).map(|s| s.name.clone()).unwrap_or_default();
                d.text_centered("Are you sure you want to remove this server?", cx, 70.0, WHITE);
                d.text_centered(&format!("'{nm}' will be lost forever! (A long time!)"), cx, 84.0, GRAY);
                let y = (h / 6.0).floor() + 96.0;
                if button(d, p, [cx - 155.0, y, 150.0, 20.0], "Delete", true) {
                    if i < self.servers.len() {
                        self.servers.remove(i);
                        self.save_servers();
                    }
                    self.sel_srv = None;
                    self.back();
                }
                if button(d, p, [cx + 5.0, y, 150.0, 20.0], "Cancel", true) {
                    self.back();
                }
                None
            }
            Screen::Connecting => {
                self.background(d, false);
                self.conn_t += dt;
                d.text_centered("Connecting to the server...", cx, 70.0, WHITE);
                let dots = ".".repeat(1 + (self.conn_t * 2.0) as usize % 3);
                d.text_centered(&format!("Logging in{dots}"), cx, 84.0, GRAY);
                let done = self.conn.as_ref().and_then(|c| c.lock().unwrap().take());
                if let Some(r) = done {
                    self.conn = None;
                    match r {
                        net::Login::Disconnected(m) => self.screen = Screen::Disc("Disconnected".into(), m),
                        net::Login::NeedsAuth => {
                            self.screen = Screen::Disc("Failed to log in".into(), "This server is in online mode and needs Mojang session authentication (encryption), which is not supported in this build yet.".into())
                        }
                        net::Login::Failed(e) => self.screen = Screen::Disc("Failed to connect to the server".into(), e),
                        net::Login::Success { name, conn } => {
                            self.joined = Some(conn);
                            self.load_msg = format!("Logged in as {name}");
                            self.screen = Screen::Loading;
                        }
                    }
                    return None;
                }
                if button(d, p, [cx - 100.0, (h / 4.0).floor() + 132.0, 200.0, 20.0], "Cancel", true) {
                    self.conn = None;
                    self.back();
                }
                None
            }
            Screen::Disc(t, m) => {
                self.background(d, false);
                self.header(d, &t, 70.0);
                let mut y = 90.0;
                for para in m.split('\n') {
                    for l in wrap(d, para, 300.0) {
                        d.text_centered(&l, cx, y, WHITE);
                        y += 12.0;
                    }
                }
                if button(d, p, [cx - 100.0, y.max((h / 6.0).floor() + 96.0 + 24.0) + 12.0, 200.0, 20.0], "Back to Server List", true) {
                    self.back();
                }
                None
            }
            Screen::Options => {
                self.background(d, false);
                self.header(d, "Options", 15.0);
                self.list_area(d, 33.0, h - 33.0);
                let (l, r) = (cx - 155.0, cx + 5.0);
                let mut y = 40.0;
                // FOV (thanh trượt) + Online
                let fov = self.get("fov");
                let (nf, _) = slider(d, p, [l, y, 150.0, 20.0], &fov_label(fov), (fov - 30.0) / 80.0, &mut self.drag, "fov");
                if let Some(v) = nf {
                    self.set.insert("fov", (30.0 + v * 80.0).round());
                    return Some(Ev::Settings);
                }
                let _ = button(d, p, [r, y, 150.0, 20.0], "Online...", false);
                y += 24.0;
                let items: [(&str, Screen); 10] = [
                    ("Skin Customization...", Screen::Page("skin")),
                    ("Music & Sounds...", Screen::Page("sound")),
                    ("Video Settings...", Screen::Page("video")),
                    ("Controls...", Screen::Page("controls")),
                    ("Language...", Screen::Page("lang")),
                    ("Chat Settings...", Screen::Page("chat")),
                    ("Resource Packs...", Screen::Page("packs")),
                    ("Accessibility Settings...", Screen::Page("access")),
                    ("Telemetry Data...", Screen::Page("telemetry")),
                    ("Credits & Attribution...", Screen::Page("credits")),
                ];
                for (i, (lab, sc)) in items.iter().enumerate() {
                    let (x, yy) = (if i % 2 == 0 { l } else { r }, y + (i / 2) as f32 * 24.0);
                    if button(d, p, [x, yy, 150.0, 20.0], lab, true) {
                        self.open(sc.clone());
                    }
                }
                if button(d, p, [cx - 100.0, h - 27.0, 200.0, 20.0], "Done", true) {
                    self.back();
                    return Some(Ev::Settings);
                }
                None
            }
            Screen::Page(pg) => self.page_screen(d, p, pg, w, h, cx),
            Screen::Pause => {
                if !self.in_game {
                    self.in_game = true;
                }
                let (bw, hw) = (204.0, 98.0);
                let top = ((h - 116.0) * 0.25).floor().max(22.0);
                d.fill(0.0, 0.0, w, h, 0x00000000);
                d.text_centered("Game Menu", cx, top - 22.0, WHITE);
                let x = cx - 102.0;
                let mut ev = None;
                if button(d, p, [x, top, bw, 20.0], "Back to Game", true) {
                    ev = Some(Ev::Resume);
                }
                let _ = button(d, p, [x, top + 24.0, hw, 20.0], "Advancements", false);
                let _ = button(d, p, [x + hw + 8.0, top + 24.0, hw, 20.0], "Statistics", false);
                let _ = button(d, p, [x, top + 48.0, hw, 20.0], "Give Feedback", false);
                let _ = button(d, p, [x + hw + 8.0, top + 48.0, hw, 20.0], "Report Bugs", false);
                if button(d, p, [x, top + 72.0, hw, 20.0], "Options...", true) {
                    self.open(Screen::Options);
                }
                let _ = button(d, p, [x + hw + 8.0, top + 72.0, hw, 20.0], "Open to LAN", false);
                if button(d, p, [x, top + 96.0, bw, 20.0], if self.online { "Disconnect" } else { "Save and Quit to Title" }, true) {
                    ev = Some(Ev::QuitToTitle);
                }
                ev
            }
            Screen::Msg(t, m) => {
                self.background(d, false);
                self.header(d, &t, 70.0);
                let lines = wrap(d, &m, 280.0);
                for (i, l) in lines.iter().enumerate() {
                    d.text_centered(l, cx, 90.0 + i as f32 * 12.0, WHITE);
                }
                if button(d, p, [cx - 100.0, (h / 6.0).floor() + 96.0 + 24.0, 200.0, 20.0], "Back", true) {
                    self.back();
                }
                None
            }
            Screen::Loading => {
                // nền đất tối + "Loading terrain..."
                d.fill(0.0, 0.0, w, h, 0x000000ff);
                let t = d.a.tr("multiplayer.downloadingTerrain");
                d.text_centered(&t, cx, h / 2.0 - 50.0, WHITE);
                if !self.load_msg.is_empty() {
                    let m = self.load_msg.clone();
                    d.text_centered(&m, cx, h / 2.0 - 36.0, GRAY);
                }
                None
            }
        }
    }

    fn multi_screen(&mut self, d: &mut Draw, p: &Ptr, w: f32, h: f32, cx: f32) -> Option<Ev> {
        self.background(d, true);
        self.header(d, "Play Multiplayer", 15.0);
        let (top, bottom) = (32.0, h - 64.0);
        self.list_area(d, top, bottom);
        let row_h = 36.0;
        let rw = 305.0;
        let rx = cx - rw / 2.0;
        let total = self.servers.len() as f32 * row_h + 4.0;
        let view = bottom - top;
        let maxs = (total - view).max(0.0);
        if p.down.map_or(false, |q| q.1 > top && q.1 < bottom) {
            self.scroll = (self.scroll - p.dy).clamp(0.0, maxs);
        }
        self.scroll = self.scroll.clamp(0.0, maxs);
        let mut join: Option<String> = None;
        d.c.set_clip(Some([0.0, top * d.k, w * d.k, view * d.k]));
        if self.servers.is_empty() {
            d.text_centered("Scanning for games on your local network", cx, top + 8.0, GRAY);
            let dots = ["O o o", "o O o", "o o O", "o O o"][((self.t * 4.0) as usize) % 4];
            d.text_centered(dots, cx, top + 19.0, GRAY);
        }
        let t = self.t;
        for i in 0..self.servers.len() {
            let y = top + 4.0 + i as f32 * row_h - self.scroll;
            if y + row_h < top || y > bottom {
                continue;
            }
            if self.sel_srv == Some(i) {
                d.fill(rx - 2.0, y - 2.0, rw + 4.0, row_h, 0xffffffff);
                d.fill(rx - 1.0, y - 1.0, rw + 2.0, row_h - 2.0, 0x000000ff);
            }
            let (name, addr) = (self.servers[i].name.clone(), self.servers[i].addr.clone());
            let mut st = self.servers[i].st.lock().unwrap();
            // icon
            if let Some(img) = st.icon.take() {
                let idx = d.c.add_image(img.w, img.h, &img.rgba);
                if idx != usize::MAX {
                    st.icon_tex = Some(idx);
                } else {
                    st.icon = Some(img);
                }
            }
            match st.icon_tex {
                Some(tx) => d.blit(tx, [rx, y, 32.0, 32.0], [0.0, 0.0, 64.0, 64.0], WHITE),
                None => {
                    if let Some(tx) = d.a.world_icon {
                        d.blit_logical(tx, [rx, y, 32.0, 32.0], [0.0, 0.0, 64.0, 64.0], 64.0, 64.0, WHITE);
                    }
                }
            }
            d.text(&name, rx + 35.0, y + 1.0, WHITE);
            let mismatch = st.state == 1 && st.protocol != 0 && st.protocol != net::PROTOCOL as i64;
            // ping / số người
            let (ping_sprite, right_text): (String, String) = match st.state {
                0 => (format!("server_list/pinging_{}", 1 + ((t * 5.0) as usize % 5)), String::new()),
                1 => {
                    let n = if st.ping_ms < 150 { 5 } else if st.ping_ms < 300 { 4 } else if st.ping_ms < 600 { 3 } else if st.ping_ms < 1000 { 2 } else { 1 };
                    let txt = if mismatch { format!("§c{}", st.version) } else { format!("§7{}§8/§7{}", st.online, st.max) };
                    (format!("server_list/ping_{n}"), txt)
                }
                _ => ("server_list/unreachable".to_string(), String::new()),
            };
            let spr = if mismatch { "server_list/incompatible".to_string() } else { ping_sprite };
            if let Some(s) = d.a.sprites.get(&spr).copied() {
                d.blit(s.tex, [rx + rw - 15.0, y, 10.0, 8.0], [0.0, 0.0, s.w, s.h], WHITE);
            }
            if !right_text.is_empty() {
                let tw = d.text_width(&right_text);
                d.text(&right_text, rx + rw - 17.0 - tw, y + 1.0, WHITE);
            }
            match st.state {
                0 => {
                    d.text("Pinging...", rx + 35.0, y + 12.0, 0x808080ff);
                }
                1 => {
                    for (li, l) in st.motd.iter().enumerate() {
                        let mut s = l.clone();
                        while d.text_width(&s) > rw - 35.0 - 3.0 && !s.is_empty() {
                            s.pop();
                        }
                        d.text(&s, rx + 35.0, y + 12.0 + li as f32 * 9.0, 0x808080ff);
                    }
                }
                _ => {
                    d.text("Can't connect to server", rx + 35.0, y + 12.0, 0xff5555ff);
                    let mut e = st.error.clone();
                    while d.text_width(&e) > rw - 38.0 && !e.is_empty() {
                        e.pop();
                    }
                    d.text(&e, rx + 35.0, y + 21.0, 0x808080ff);
                }
            }
            drop(st);
            let _ = addr.len();
            if let Some(tp) = p.tap {
                if tp.0 >= rx && tp.0 < rx + rw && tp.1 >= y && tp.1 < y + row_h && tp.1 > top && tp.1 < bottom {
                    if self.sel_srv == Some(i) && self.t - self.last_tap.1 < 0.5 && self.last_tap.0 == i {
                        join = Some(addr);
                    }
                    self.sel_srv = Some(i);
                    self.last_tap = (i, self.t);
                }
            }
        }
        d.c.set_clip(None);
        if maxs > 0.0 {
            let sx = cx + rw / 2.0 + 6.0;
            let bh = ((view * view) / total).clamp(32.0, view - 8.0);
            let by = top + (view - bh) * (self.scroll / maxs);
            d.sprite("widget/scroller_background", sx, top, 6.0, view);
            d.sprite("widget/scroller", sx, by, 6.0, bh);
        }
        let (y1, y2) = (h - 52.0, h - 28.0);
        let have = self.sel_srv.is_some();
        if button(d, p, [cx - 154.0, y1, 100.0, 20.0], "Join Server", have) {
            if let Some(i) = self.sel_srv {
                join = Some(self.servers[i].addr.clone());
            }
        }
        if button(d, p, [cx - 50.0, y1, 100.0, 20.0], "Direct Connection", true) {
            self.edit_addr.clear();
            self.focus = 1;
            self.open(Screen::Direct);
        }
        if button(d, p, [cx + 54.0, y1, 100.0, 20.0], "Add Server", true) {
            self.edit_idx = None;
            self.edit_name = "Minecraft Server".into();
            self.edit_addr.clear();
            self.focus = 1;
            self.open(Screen::EditServer);
        }
        if button(d, p, [cx - 154.0, y2, 70.0, 20.0], "Edit", have) {
            if let Some(i) = self.sel_srv {
                self.edit_idx = Some(i);
                self.edit_name = self.servers[i].name.clone();
                self.edit_addr = self.servers[i].addr.clone();
                self.focus = 0;
                self.open(Screen::EditServer);
            }
        }
        if button(d, p, [cx - 74.0, y2, 70.0, 20.0], "Delete", have) {
            if let Some(i) = self.sel_srv {
                self.open(Screen::ConfirmDelete(i));
            }
        }
        if button(d, p, [cx + 4.0, y2, 70.0, 20.0], "Refresh", true) {
            self.refresh_servers();
        }
        if button(d, p, [cx + 84.0, y2, 70.0, 20.0], "Back", true) {
            self.back();
        }
        if let Some(a) = join {
            self.join(a);
        }
        None
    }

    /// Màn sửa server / kết nối trực tiếp, kèm bàn phím trong game.
    fn edit_screen(&mut self, d: &mut Draw, p: &Ptr, w: f32, h: f32, cx: f32, direct: bool) -> Option<Ev> {
        let _ = w;
        self.background(d, false);
        self.header(d, if direct { "Direct Connection" } else { "Edit Server Info" }, 10.0);
        let mut y = 24.0;
        if !direct {
            d.text("Server Name", cx - 100.0, y, 0xa0a0a0ff);
            y += 11.0;
            let r = [cx - 100.0, y, 200.0, 20.0];
            self.field(d, p, r, 0);
            y += 24.0;
        }
        d.text("Server Address", cx - 100.0, y, 0xa0a0a0ff);
        y += 11.0;
        let r = [cx - 100.0, y, 200.0, 20.0];
        self.field(d, p, r, 1);
        y += 26.0;
        let ok = !self.edit_addr.trim().is_empty() && (direct || !self.edit_name.trim().is_empty());
        let (lab_ok, lab_no) = if direct { ("Join Server", "Cancel") } else { ("Done", "Cancel") };
        if button(d, p, [cx - 100.0, y, 98.0, 20.0], lab_ok, ok) {
            if direct {
                let a = self.edit_addr.trim().to_string();
                self.back();
                self.join(a);
            } else {
                let (n, a) = (self.edit_name.trim().to_string(), self.edit_addr.trim().to_string());
                match self.edit_idx {
                    Some(i) if i < self.servers.len() => {
                        self.servers[i].name = n;
                        self.servers[i].addr = a;
                        self.save_servers();
                        self.ping_one(i);
                    }
                    _ => {
                        self.servers.push(Server { name: n, addr: a, st: Arc::new(Mutex::new(net::Status::default())) });
                        self.save_servers();
                        let i = self.servers.len() - 1;
                        self.ping_one(i);
                    }
                }
                self.back();
            }
            return None;
        }
        if button(d, p, [cx + 2.0, y, 98.0, 20.0], lab_no, true) {
            self.back();
            return None;
        }
        y += 28.0;
        // bàn phím
        let rows = ["1234567890", "qwertyuiop", "asdfghjkl"];
        let kw = 19.0;
        let mut ty = y;
        for row in rows {
            let x0 = cx - row.len() as f32 * kw / 2.0;
            for (i, c) in row.chars().enumerate() {
                let ch = if self.shift { c.to_ascii_uppercase() } else { c };
                if button(d, p, [x0 + i as f32 * kw, ty, kw - 2.0, 18.0], &ch.to_string(), true) {
                    self.type_char(ch, direct);
                }
            }
            ty += 20.0;
        }
        let row = "zxcvbnm";
        let x0 = cx - (row.len() as f32 * kw + 2.0 * 28.0) / 2.0;
        if button(d, p, [x0, ty, 26.0, 18.0], "^", true) {
            self.shift = !self.shift;
        }
        for (i, c) in row.chars().enumerate() {
            let ch = if self.shift { c.to_ascii_uppercase() } else { c };
            if button(d, p, [x0 + 28.0 + i as f32 * kw, ty, kw - 2.0, 18.0], &ch.to_string(), true) {
                self.type_char(ch, direct);
            }
        }
        if button(d, p, [x0 + 28.0 + row.len() as f32 * kw, ty, 26.0, 18.0], "<", true) {
            self.buf(direct).pop();
        }
        ty += 20.0;
        let specials = [(".", 1.0), (":", 1.0), ("-", 1.0), ("_", 1.0), (" ", 4.0), ("/", 1.0)];
        let total: f32 = specials.iter().map(|s| s.1 * kw).sum();
        let mut x = cx - total / 2.0;
        for (s, wd) in specials {
            let lab = if s == " " { "space" } else { s };
            if button(d, p, [x, ty, wd * kw - 2.0, 18.0], lab, true) {
                self.type_char(s.chars().next().unwrap(), direct);
            }
            x += wd * kw;
        }
        None
    }

    fn buf(&mut self, direct: bool) -> &mut String {
        if direct || self.focus == 1 {
            &mut self.edit_addr
        } else {
            &mut self.edit_name
        }
    }

    fn type_char(&mut self, c: char, direct: bool) {
        let b = self.buf(direct);
        if b.chars().count() < 128 {
            b.push(c);
        }
        self.shift = false;
    }

    fn field(&mut self, d: &mut Draw, p: &Ptr, r: [f32; 4], idx: usize) {
        if inside(r, p.tap) {
            self.focus = idx;
        }
        let on = self.focus == idx;
        d.sprite(if on && d.has_sprite("widget/text_field_highlighted") { "widget/text_field_highlighted" } else { "widget/text_field" }, r[0], r[1], r[2], r[3]);
        let mut s = if idx == 0 { self.edit_name.clone() } else { self.edit_addr.clone() };
        let cur = on && (self.t * 2.0) as i32 % 2 == 0;
        // cuộn: hiện phần cuối chuỗi
        while d.text_width(&s) > r[2] - 12.0 && !s.is_empty() {
            s.remove(0);
        }
        let ex = d.text(&s, r[0] + 4.0, r[1] + 6.0, 0xe0e0e0ff);
        if cur {
            d.text("_", ex, r[1] + 6.0, 0xe0e0e0ff);
        }
    }

    fn worlds_screen(&mut self, d: &mut Draw, p: &Ptr, w: f32, h: f32, cx: f32) -> Option<Ev> {
        self.background(d, true);
        let (top, bottom) = (48.0, h - 64.0);
        self.list_area(d, top, bottom);
        let mut ev = None;
        // kéo cuộn
        let row_h = 36.0;
        let total = self.worlds.len() as f32 * row_h + 4.0;
        let view = bottom - top;
        let maxs = (total - view).max(0.0);
        if p.down.map_or(false, |q| q.1 > top && q.1 < bottom) {
            self.scroll = (self.scroll - p.dy).clamp(0.0, maxs);
        }
        self.scroll = self.scroll.clamp(0.0, maxs);
        let title = d.a.tr("selectWorld.title");
        self.header(d, &title, 8.0);
        // ô tìm kiếm
        d.sprite("widget/text_field", cx - 100.0, 22.0, 200.0, 20.0);
        let sr = d.a.tr("selectWorld.search");
        d.text(&sr, cx - 94.0, 28.0, 0x707070ff);
        // danh sách
        d.c.set_clip(Some([0.0, top * d.k, w * d.k, view * d.k]));
        let rx = cx - 135.0;
        if self.worlds.is_empty() {
            let m = self.worlds_err.clone().unwrap_or_else(|| "No worlds found. Put a world folder into /sdcard/RustVoxel.".into());
            let lines = wrap(d, &m, 300.0);
            for (i, l) in lines.iter().enumerate() {
                d.text_centered(l, cx, top + 20.0 + i as f32 * 12.0, GRAY);
            }
        }
        let tz = self.tz_min;
        for i in 0..self.worlds.len() {
            let y = top + 4.0 + i as f32 * row_h - self.scroll;
            if y + row_h < top || y > bottom {
                continue;
            }
            let wd = self.worlds[i].clone();
            if self.sel == Some(i) {
                d.fill(rx - 2.0, y - 2.0, 274.0, row_h + 0.0 + 0.0, 0xffffffff);
                d.fill(rx - 1.0, y - 1.0, 272.0, row_h - 2.0, 0x000000ff);
            }
            if let Some(t) = d.a.world_icon {
                d.blit_logical(t, [rx, y, 32.0, 32.0], [0.0, 0.0, 64.0, 64.0], 64.0, 64.0, WHITE);
            }
            d.text(&wd.name, rx + 35.0, y + 1.0, WHITE);
            let date = fmt_date(wd.last_played, tz);
            let l2 = if date.is_empty() { wd.folder.clone() } else { format!("{} ({})", wd.folder, date) };
            d.text(&l2, rx + 35.0, y + 12.0, 0x808080ff);
            let modes = ["selectWorld.gameMode.survival", "selectWorld.gameMode.creative", "selectWorld.gameMode.adventure", "selectWorld.gameMode.spectator"];
            let mut l3 = if wd.hardcore { "§4§lHardcore Mode§r".to_string() } else { d.a.tr(modes[(wd.game_mode.clamp(0, 3)) as usize]) };
            if wd.hardcore {
                l3 = format!("§4{}", d.a.tr("selectWorld.gameMode.hardcore"));
            }
            if wd.cheats {
                l3 = format!("{}, {}", l3, d.a.tr("selectWorld.cheats"));
            }
            if !wd.version.is_empty() {
                l3 = format!("{}, {} {}", l3, d.a.tr("selectWorld.version"), wd.version);
            }
            d.text(&l3, rx + 35.0, y + 23.0, 0x808080ff);
            if let Some(t) = p.tap {
                if t.0 >= rx && t.0 < rx + 270.0 && t.1 >= y && t.1 < y + row_h && t.1 > top && t.1 < bottom {
                    if self.sel == Some(i) && self.t - self.last_tap.1 < 0.5 && self.last_tap.0 == i {
                        ev = Some(Ev::Play(i));
                    }
                    self.sel = Some(i);
                    self.last_tap = (i, self.t);
                }
            }
        }
        d.c.set_clip(None);
        // thanh cuộn
        if maxs > 0.0 {
            let sx = cx + 135.0 + 6.0;
            let bh = ((view * view) / total).clamp(32.0, view - 8.0);
            let by = top + (view - bh) * (self.scroll / maxs);
            d.sprite("widget/scroller_background", sx, top, 6.0, view);
            d.sprite("widget/scroller", sx, by, 6.0, bh);
        }
        // nút
        let (y1, y2) = (h - 52.0, h - 28.0);
        let have = self.sel.is_some();
        let play = d.a.tr("selectWorld.select");
        if button(d, p, [cx - 154.0, y1, 150.0, 20.0], &play, have) {
            if let Some(i) = self.sel {
                ev = Some(Ev::Play(i));
            }
        }
        let cr = d.a.tr("selectWorld.create");
        if button(d, p, [cx + 4.0, y1, 150.0, 20.0], &cr, true) {
            self.open(Screen::Create);
        }
        let (e, dl, rc, bk) = (d.a.tr("selectWorld.edit"), d.a.tr("selectWorld.delete"), d.a.tr("selectWorld.recreate"), d.a.tr("gui.back"));
        let _ = button(d, p, [cx - 154.0, y2, 72.0, 20.0], &e, false);
        let _ = button(d, p, [cx - 76.0, y2, 72.0, 20.0], &dl, false);
        let _ = button(d, p, [cx + 4.0, y2, 72.0, 20.0], &rc, false);
        if button(d, p, [cx + 82.0, y2, 72.0, 20.0], &bk, true) {
            self.back();
        }
        ev
    }

    fn page_screen(&mut self, d: &mut Draw, p: &Ptr, pg: &'static str, w: f32, h: f32, cx: f32) -> Option<Ev> {
        self.background(d, false);
        let (top, bottom) = (33.0, h - 33.0);
        self.list_area(d, top, bottom);
        let ops = page_opts(pg);
        let mut ev = None;
        // nội dung chữ cho trang không có tuỳ chọn
        if ops.is_empty() {
            let lines: Vec<String> = match pg {
                "lang" => vec!["English (US)".into(), "".into(), "Only English is available in this build.".into()],
                "packs" => vec!["Only the default resource pack is available.".into(), "".into(), "Assets are read from your client.jar.".into()],
                "telemetry" => vec!["Rust Voxel does not collect or send any telemetry.".into()],
                "credits" => vec!["Minecraft is a trademark of Mojang Studios.".into(), "Textures, models and language files are read".into(), "from your own installed client.jar.".into(), "".into(), "Rust Voxel is an unofficial fan project.".into()],
                _ => vec![],
            };
            for (i, l) in lines.iter().enumerate() {
                d.text_centered(l, cx, top + 20.0 + i as f32 * 12.0, if pg == "lang" && i == 0 { WHITE } else { GRAY });
            }
        } else {
            let rows = {
                let mut r = 0usize;
                let mut col = 0;
                for op in ops {
                    if op.full {
                        if col == 1 {
                            r += 1;
                            col = 0;
                        }
                        r += 1;
                    } else {
                        col += 1;
                        if col == 2 {
                            r += 1;
                            col = 0;
                        }
                    }
                }
                if col == 1 {
                    r += 1;
                }
                r
            };
            let total = rows as f32 * 24.0 + 8.0;
            let view = bottom - top;
            let maxs = (total - view).max(0.0);
            if p.down.map_or(false, |q| q.1 > top && q.1 < bottom) && self.drag.is_none() {
                self.scroll = (self.scroll - p.dy).clamp(0.0, maxs);
            }
            d.c.set_clip(Some([0.0, top * d.k, w * d.k, view * d.k]));
            let (mut r, mut col) = (0usize, 0usize);
            for op in ops {
                if op.full && col == 1 {
                    r += 1;
                    col = 0;
                }
                let (x, wd) = if op.full { (cx - 155.0, 310.0) } else { (if col == 0 { cx - 155.0 } else { cx + 5.0 }, 150.0) };
                let y = top + 4.0 + r as f32 * 24.0 - self.scroll;
                let key = op.key;
                let v = self.get(key);
                if y + 20.0 > top && y < bottom {
                    match op.kind {
                        Kind::Slider(mn, mx, st) => {
                            let (nv, _) = slider(d, p, [x, y, wd, 20.0], &fmt_val(op, v), (v - mn) / (mx - mn), &mut self.drag, key);
                            if let Some(f) = nv {
                                let val = mn + f * (mx - mn);
                                let val = if st > 0.0 { (val / st).round() * st } else { val };
                                self.set.insert(key, val.clamp(mn, mx));
                                ev = Some(Ev::Settings);
                            }
                        }
                        Kind::Cycle(l) => {
                            if button(d, p, [x, y, wd, 20.0], &fmt_val(op, v), true) {
                                let n = (v as usize + 1) % l.len();
                                self.set.insert(key, n as f32);
                                ev = Some(Ev::Settings);
                            }
                        }
                        Kind::Toggle => {
                            if button(d, p, [x, y, wd, 20.0], &fmt_val(op, v), true) {
                                self.set.insert(key, if v > 0.5 { 0.0 } else { 1.0 });
                                ev = Some(Ev::Settings);
                            }
                        }
                    }
                }
                if op.full {
                    r += 1;
                } else {
                    col += 1;
                    if col == 2 {
                        col = 0;
                        r += 1;
                    }
                }
            }
            d.c.set_clip(None);
            if maxs > 0.0 {
                let sx = cx + 165.0;
                let bh = ((view * view) / total).clamp(32.0, view - 8.0);
                let by = top + (view - bh) * (self.scroll / maxs);
                d.sprite("widget/scroller_background", sx, top, 6.0, view);
                d.sprite("widget/scroller", sx, by, 6.0, bh);
            }
            self.scroll = self.scroll.clamp(0.0, maxs);
        }
        self.header(d, page_title(pg), 15.0);
        if button(d, p, [cx - 100.0, h - 27.0, 200.0, 20.0], "Done", true) {
            self.back();
            ev = Some(Ev::Settings);
        }
        ev
    }
}

fn fov_label(v: f32) -> String {
    if v == 70.0 {
        "FOV: Normal".into()
    } else if v >= 110.0 {
        "FOV: Quake Pro".into()
    } else {
        format!("FOV: {}", v as i32)
    }
}

/// Thanh trượt vanilla. Trả về Some(giá trị 0..1) khi đang kéo.
fn slider(d: &mut Draw, p: &Ptr, r: [f32; 4], label: &str, v: f32, drag: &mut Option<&'static str>, id: &'static str) -> (Option<f32>, bool) {
    if let Some(q) = p.press {
        if inside(r, Some(q)) {
            *drag = Some(id);
        }
    }
    let active = *drag == Some(id);
    let hot = active || inside(r, p.down);
    d.sprite(if hot { "widget/slider_highlighted" } else { "widget/slider" }, r[0], r[1], r[2], r[3]);
    let v = v.clamp(0.0, 1.0);
    let hx = r[0] + (r[2] - 8.0) * v;
    d.sprite(if hot { "widget/slider_handle_highlighted" } else { "widget/slider_handle" }, hx, r[1], 8.0, r[3]);
    d.text_centered(label, r[0] + r[2] / 2.0, r[1] + 6.0, if hot { YELLOW } else { WHITE });
    let mut out = None;
    if active {
        if let Some(q) = p.down {
            out = Some(((q.0 - r[0] - 4.0) / (r[2] - 8.0)).clamp(0.0, 1.0));
        }
    }
    (out, active)
}

/// ngắt dòng theo độ rộng GUI
pub fn wrap(d: &Draw, s: &str, maxw: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split(' ') {
        let t = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if d.text_width(&t) > maxw && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur = word.to_string();
        } else {
            cur = t;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
