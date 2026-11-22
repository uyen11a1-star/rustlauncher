//! Màn hình xem cảnh 3D (G1): renderer GLES3 tự viết + điều khiển cảm ứng + HUD.
use std::{
    collections::HashMap,
    ffi::CString,
    sync::atomic::Ordering::Relaxed,
    time::Instant,
};

use crate::{
    assets::V,
    gfx::{build_program, Gfx},
    gles::*,
    inv::{Inv, SLOTS},
    items::{self, Hud, Item, Spr, SWING_T},
    math,
    phys::{Collider, Input, Player, TICK},
    scene::{bright, pack, ChunkMesh, Msg, SceneData, Streamer},
    world::World,
};

struct Gpu {
    vbo: u32,
    ebo: u32,
    count: i32,
    cw: i32,
    nverts: usize,
    bb: [f32; 6],
}

/// Đưa mesh một chunk lên GPU. Cần context GL đang hiện hành.
unsafe fn upload(c: &ChunkMesh) -> Gpu {
    let (mut vbo, mut ebo) = (0, 0);
    glGenBuffers(1, &mut vbo);
    glBindBuffer(GL_ARRAY_BUFFER, vbo);
    glBufferData(GL_ARRAY_BUFFER, (c.v.len() * std::mem::size_of::<V>()) as isize, c.v.as_ptr() as *const _, GL_STATIC_DRAW);
    glGenBuffers(1, &mut ebo);
    glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ebo);
    let mut idx = c.i.clone();
    idx.extend_from_slice(&c.iw);
    glBufferData(GL_ELEMENT_ARRAY_BUFFER, (idx.len() * 4) as isize, idx.as_ptr() as *const _, GL_STATIC_DRAW);
    Gpu { vbo, ebo, count: c.i.len() as i32, cw: c.iw.len() as i32, nverts: c.v.len(), bb: c.bb }
}

/// Đưa một lưới đỉnh/chỉ số bất kỳ lên GPU (vật phẩm, tay).
unsafe fn upload_raw(v: &[V], i: &[u32]) -> Gpu {
    let (mut vbo, mut ebo) = (0, 0);
    glGenBuffers(1, &mut vbo);
    glBindBuffer(GL_ARRAY_BUFFER, vbo);
    glBufferData(GL_ARRAY_BUFFER, (v.len() * std::mem::size_of::<V>()) as isize, v.as_ptr() as *const _, GL_STATIC_DRAW);
    glGenBuffers(1, &mut ebo);
    glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ebo);
    glBufferData(GL_ELEMENT_ARRAY_BUFFER, (i.len() * 4) as isize, i.as_ptr() as *const _, GL_STATIC_DRAW);
    Gpu { vbo, ebo, count: i.len() as i32, cw: 0, nverts: v.len(), bb: [0.0; 6] }
}

unsafe fn make_tex(rgba: &[u8], w: i32, h: i32) -> u32 {
    let mut tex = 0;
    glGenTextures(1, &mut tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, rgba.as_ptr() as *const _);
    for (p, v) in [(GL_TEXTURE_MIN_FILTER, GL_NEAREST), (GL_TEXTURE_MAG_FILTER, GL_NEAREST), (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE), (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE)] {
        glTexParameteri(GL_TEXTURE_2D, p, v);
    }
    tex
}

const MAP_COLORS: [[u8; 3]; 62] = [
    [0, 0, 0], [127, 178, 56], [247, 233, 163], [199, 199, 199], [255, 0, 0], [160, 160, 255], [167, 167, 167], [0, 124, 0],
    [255, 255, 255], [164, 168, 184], [151, 109, 77], [112, 112, 112], [64, 64, 255], [143, 119, 72], [255, 252, 245], [216, 127, 51],
    [178, 76, 216], [102, 153, 216], [229, 229, 51], [127, 204, 25], [242, 127, 165], [76, 76, 76], [153, 153, 153], [76, 127, 153],
    [127, 63, 178], [51, 76, 178], [102, 76, 51], [102, 127, 51], [153, 51, 51], [25, 25, 25], [250, 238, 77], [92, 219, 213],
    [74, 128, 255], [0, 217, 58], [129, 86, 49], [112, 2, 0], [209, 177, 161], [159, 82, 36], [149, 87, 108], [112, 108, 138],
    [186, 133, 36], [103, 117, 53], [160, 77, 78], [57, 41, 35], [135, 107, 98], [87, 92, 92], [122, 73, 88], [76, 62, 92],
    [76, 50, 35], [76, 82, 42], [142, 60, 46], [37, 22, 16], [189, 48, 49], [148, 63, 97], [92, 25, 29], [22, 126, 134],
    [58, 142, 140], [86, 44, 62], [20, 180, 133], [100, 100, 100], [216, 175, 147], [127, 167, 150],
];

unsafe fn free(g: &Gpu) {
    glDeleteBuffers(1, &g.vbo);
    glDeleteBuffers(1, &g.ebo);
}

const VS: &str = "#version 300 es
in vec3 a_pos; in vec2 a_uv; in vec4 a_col;
uniform mat4 u_mvp;
out vec2 v_uv; out vec4 v_col; out float v_d;
void main(){ v_uv=a_uv; v_col=a_col; vec4 p=u_mvp*vec4(a_pos,1.0); v_d=p.w; gl_Position=p; }";

const FS: &str = "#version 300 es
precision highp float;
in vec2 v_uv; in vec4 v_col; in float v_d;
uniform sampler2D u_tex;
uniform vec2 u_fog;
uniform vec3 u_fogc;
uniform vec3 u_tint;
out vec4 o;
void main(){
  vec4 t=texture(u_tex,v_uv); if(t.a<0.1) discard;
  float f=clamp((u_fog.y-v_d)/(u_fog.y-u_fog.x),0.0,1.0);
  vec3 c=mix(u_fogc, t.rgb*v_col.rgb*u_tint, f);
  o=vec4(c, t.a*v_col.a);
}";

const VS_SKY: &str = "#version 300 es
out vec2 v_ndc;
void main(){ vec2 p=vec2(float((gl_VertexID<<1)&2), float(gl_VertexID&2)); v_ndc=p*2.0-1.0; gl_Position=vec4(v_ndc,1.0,1.0); }";

const FS_SKY: &str = "#version 300 es
precision highp float;
in vec2 v_ndc;
uniform mat4 u_view;
uniform vec2 u_tf;
out vec4 o;
void main(){
  vec3 d=normalize(vec3(v_ndc.x*u_tf.x, v_ndc.y*u_tf.y, -1.0))*mat3(u_view);
  float h=clamp(d.y,0.0,1.0);
  vec3 c=mix(vec3(0.53,0.81,0.92), vec3(0.25,0.50,0.88), pow(h,0.6));
  float k=dot(d, normalize(vec3(0.5,0.7,0.4)));
  c+=vec3(1.0,0.95,0.7)*smoothstep(0.9985,0.9995,k);
  c+=vec3(1.0,0.9,0.6)*0.25*pow(max(k,0.0),64.0);
  o=vec4(c,1.0);
}";

/// Sương mù: kín hoàn toàn trước mép vùng nạp (tầm stream 6 chunk = 96 block, xem launcher.rs).
const FOG_START: f32 = 40.0;
const FOG_END: f32 = 90.0;

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Move,
    Look,
    Jump,
    Sneak,
    Fly,
    Sprint,
    Exit,
    Bag,
    Chat,
    Attack,
    Place,
    /// ô thanh nhanh số i (khi túi đóng)
    Hot(usize),
    /// chạm trong màn hình túi đồ
    Ui,
}

struct Touch {
    id: i32,
    role: Role,
    sx: f32,
    sy: f32,
    x: f32,
    y: f32,
    t0: Instant,
    moved: f32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Phase {
    Down,
    Move,
    Up,
}

pub struct View {
    prog: u32,
    sky: u32,
    sky_view: i32,
    sky_tf: i32,
    u_fog: i32,
    u_fogc: i32,
    u_tint: i32,
    inv_tex: Option<(u32, f32, f32)>,
    chest_tex: Option<(u32, f32, f32)>,
    /// cửa sổ vật chứa server đang mở (rương, menu chọn chế độ chơi)
    win: Option<crate::netplay::NetWin>,
    /// vị trí ngón tay trên cửa sổ vật chứa (để hiện tên vật)
    win_pt: Option<(f32, f32)>,
    hot_tex: Option<(u32, f32, f32, [f32; 4])>,
    sel_tex: Option<(u32, f32, f32, [f32; 4])>,
    body_gpu: Gpu,
    head_n: i32,
    hud: Option<Hud>,
    hud_tex: Vec<(u32, f32, f32)>,
    health: f32,
    fall: f32,
    hurt_t: f32,
    regen_t: f32,
    quad_vbo: u32,
    items: Vec<Item>,
    items_gpu: Gpu,
    arm_gpu: Gpu,
    map_gpu: Gpu,
    map_tex: u32,
    map_idx: i32,
    map_has: bool,
    skin_tex: u32,
    inv: Inv,
    inv_open: bool,
    ui_pt: (f32, f32),
    /// (ô chạm xuống, đã nhấc vật lên lúc chạm xuống)
    ui_down: Option<(usize, bool)>,
    swing: Option<f32>,
    lower: f32,
    shown: Option<u16>,
    hand_b: f32,
    name_t: f32,
    last_held: (usize, Option<u16>),
    gpu: HashMap<(i32, i32), Gpu>,
    stream: Option<Streamer>,
    tex: u32,
    u_mvp: i32,
    u_tex: i32,
    nverts: usize,
    drawn_chunks: usize,
    drawn_verts: usize,
    /// vị trí mắt (đã nội suy) — dùng cho camera, stream và HUD
    pos: [f32; 3],
    col: Option<Collider>,
    player: Player,
    prev: [f64; 3],
    prev_eye: f64,
    acc: f32,
    spawn: [f64; 3],
    jump_held: bool,
    sneak_on: bool,
    sprint_on: bool,
    yaw: f32,
    pitch: f32,
    touches: Vec<Touch>,
    last: Instant,
    fps_t: Instant,
    frames: u32,
    fps: u32,
    exit: bool,
    /// góc nhìn dọc (độ); đặt từ Options
    pub fov_deg: f32,
    /// tạm dừng: không nhận chạm, không cập nhật vật lý
    pub frozen: bool,
    /// chế độ nhiều người chơi: kênh gửi vị trí
    net: Option<std::sync::mpsc::Sender<crate::netplay::NetOut>>,
    kicked: Option<String>,
    net_state: Option<crate::netplay::SharedState>,
    /// các dòng chat đã nhận (đã đổi mã §) kèm lúc nhận
    chat_log: Vec<(String, Instant)>,
    chat_open: bool,
    chat_text: String,
    chat_tap: Option<(f32, f32)>,
    kb: crate::keyboard::Keyboard,
    keys_in: Vec<KeyIn>,
    sent_sel: usize,
    key_names: Vec<String>,
    attack_held: bool,
    /// số gói đào/đặt đã gửi và mô tả mục tiêu đang ngắm (hiện ở giữa màn hình để dò lỗi)
    sent: i32,
    aim_dbg: String,
    dig: Option<Dig>,
    place_req: bool,
    others: HashMap<i32, Other>,
    /// tên người chơi trên màn hình: (x, y, tên)
    tags: Vec<(f32, f32, String)>,
}

/// Người chơi khác, vị trí hiển thị được làm mượt.
struct Other {
    pos: [f32; 3],
    tgt: [f64; 3],
    yaw: f32,
    pitch: f32,
    head: f32,
    name: String,
}

fn lerp_ang(a: f32, b: f32, t: f32) -> f32 {
    let mut d = (b - a) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    a + d * t
}

/// Phím từ bàn phím hệ thống.
#[derive(Clone, Copy)]
pub enum KeyIn {
    Char(char),
    Back,
    Enter,
    Esc,
}

/// Nút Đánh/Đặt nằm ngay bên trái cụm Nhảy/Lén (tay phải), không đè lên nút nào khác.
fn rect_attack(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 184.0 * s, g.h as f32 - 200.0 * s, 88.0 * s, 76.0 * s]
}
fn rect_place(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 184.0 * s, g.h as f32 - 116.0 * s, 88.0 * s, 48.0 * s]
}

/// Độ cứng khối (tên -> (độ cứng, cần dụng cụ)), từ hardness.txt.
fn hardness(name: &str) -> (f32, bool) {
    static T: std::sync::OnceLock<HashMap<&'static str, (f32, bool)>> = std::sync::OnceLock::new();
    let t = T.get_or_init(|| {
        include_str!("hardness.txt")
            .lines()
            .filter_map(|l| {
                let mut p = l.split(' ');
                Some((p.next()?, (p.next()?.parse().ok()?, p.next()? == "1")))
            })
            .collect()
    });
    t.get(name).copied().unwrap_or((1.0, false))
}

struct Aim {
    pos: [i32; 3],
    face: u8,
    cur: [f32; 3],
}

/// Đang giữ nút Đánh vào khối nào.
struct Dig {
    pos: [i32; 3],
    t0: Instant,
    /// thời gian đào ước lượng (giây)
    need: f32,
    finished: Option<Instant>,
}

fn rect_chat(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 92.0 * s, 132.0 * s, 80.0 * s, 40.0 * s]
}

/// Màu mã § của Minecraft.
fn mc_color(c: char) -> Option<u32> {
    Some(match c {
        '0' => 0x000000,
        '1' => 0x0000aa,
        '2' => 0x00aa00,
        '3' => 0x00aaaa,
        '4' => 0xaa0000,
        '5' => 0xaa00aa,
        '6' => 0xffaa00,
        '7' => 0xaaaaaa,
        '8' => 0x555555,
        '9' => 0x5555ff,
        'a' => 0x55ff55,
        'b' => 0x55ffff,
        'c' => 0xff5555,
        'd' => 0xff55ff,
        'e' => 0xffff55,
        'f' => 0xffffff,
        _ => return None,
    })
}

/// Cắt một dòng có mã § thành các dòng vừa `maxw` pixel, giữ màu khi xuống dòng.
fn wrap_legacy(g: &mut Gfx, text: &str, size: f32, maxw: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut plain = String::new();
    let mut col = String::new();
    let mut it = text.chars().peekable();
    let mut word = String::new();
    let mut wplain = String::new();
    let mut flush = |g: &mut Gfx, word: &mut String, wplain: &mut String, cur: &mut String, plain: &mut String, lines: &mut Vec<String>, col: &str| {
        if word.is_empty() {
            return;
        }
        let try_w = g.text_width(&format!("{plain}{wplain}"), size);
        if try_w > maxw && !plain.is_empty() {
            lines.push(std::mem::take(cur));
            plain.clear();
            cur.push_str(col);
        }
        cur.push_str(word);
        plain.push_str(wplain);
        word.clear();
        wplain.clear();
    };
    while let Some(c) = it.next() {
        if c == '§' {
            if let Some(n) = it.next() {
                word.push('§');
                word.push(n);
                if n == 'r' {
                    col.clear();
                } else if mc_color(n).is_some() {
                    col = format!("§{n}");
                }
            }
        } else if c == ' ' {
            word.push(c);
            wplain.push(c);
            let cl = col.clone();
            flush(g, &mut word, &mut wplain, &mut cur, &mut plain, &mut lines, &cl);
        } else {
            word.push(c);
            wplain.push(c);
        }
    }
    let cl = col.clone();
    flush(g, &mut word, &mut wplain, &mut cur, &mut plain, &mut lines, &cl);
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Vẽ chuỗi có mã § (màu) với bóng đổ; `a` là độ đậm 0..255.
fn draw_legacy(g: &mut Gfx, x: f32, y: f32, size: f32, text: &str, a: u32) {
    let mut col = 0xffffffu32;
    let mut pen = x;
    let mut seg = String::new();
    let mut it = text.chars();
    let mut emit = |g: &mut Gfx, seg: &mut String, col: u32, pen: &mut f32| {
        if seg.is_empty() {
            return;
        }
        let sh = ((col >> 2) & 0x3f3f3f) << 8 | a;
        g.text(*pen + size / 9.0, y + size / 9.0, size, sh, seg);
        *pen += g.text(*pen, y, size, col << 8 | a, seg);
        seg.clear();
    };
    while let Some(c) = it.next() {
        if c == '§' {
            emit(g, &mut seg, col, &mut pen);
            match it.next() {
                Some('r') => col = 0xffffff,
                Some(n) => {
                    if let Some(c) = mc_color(n) {
                        col = c;
                    }
                }
                None => {}
            }
        } else {
            seg.push(c);
        }
    }
    emit(g, &mut seg, col, &mut pen);
}

/// Số lượng ở góc dưới-phải ô vật phẩm 16x16 (có bóng xám như vanilla), ẩn khi chỉ có 1.
fn draw_count(g: &mut Gfx, r: [f32; 4], n: u8) {
    if n <= 1 {
        return;
    }
    let k = r[2] / 16.0;
    let size = 9.0 * k;
    let txt = n.to_string();
    let tw = g.text_width(&txt, size);
    let (x, y) = (r[0] + 17.0 * k - tw, r[1] + 17.0 * k - size * 1.25);
    g.text(x + k, y + k, size, 0x3f3f3fff, &txt);
    g.text(x, y, size, 0xffffffff, &txt);
}

fn rect_bag(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 92.0 * s, 84.0 * s, 80.0 * s, 40.0 * s]
}

/// Hệ số phóng GUI nguyên như Minecraft ("GUI scale auto"): lớn nhất mà vẫn còn ≥ 320x240 đơn vị.
fn gs(g: &Gfx) -> f32 {
    ((g.w as f32 / 320.0).floor().min((g.h as f32 / 240.0).floor())).max(1.0)
}

/// Góc trên-trái ảnh thanh nhanh (182x22 đơn vị GUI, sát đáy giữa màn hình).
fn hot_origin(g: &Gfx) -> (f32, f32) {
    let k = gs(g);
    ((g.w as f32 - 182.0 * k) / 2.0, g.h as f32 - 22.0 * k)
}

/// Ô vật phẩm 16x16 của ô thanh nhanh i.
fn hot_item_rect(g: &Gfx, i: usize) -> [f32; 4] {
    let (bx, by) = hot_origin(g);
    let k = gs(g);
    [bx + (3.0 + 20.0 * i as f32) * k, by + 3.0 * k, 16.0 * k, 16.0 * k]
}

fn hot_slot_rect(g: &Gfx, i: usize) -> [f32; 4] {
    let (bx, by) = hot_origin(g);
    let k = gs(g);
    [bx + 20.0 * i as f32 * k, by, 20.0 * k, 22.0 * k]
}

/// Khung túi đồ 176x166 đơn vị GUI, giữa màn hình: [x, y, rộng, cao].
fn inv_panel(g: &Gfx) -> [f32; 4] {
    let k = gs(g);
    [(g.w as f32 - 176.0 * k) / 2.0, (g.h as f32 - 166.0 * k) / 2.0, 176.0 * k, 166.0 * k]
}

/// Ô vật phẩm 16x16 của ô túi i (0..8 thanh nhanh, 9..35 túi) theo bố cục inventory.png của Minecraft.
fn inv_item_rect(g: &Gfx, i: usize) -> [f32; 4] {
    let p = inv_panel(g);
    let k = gs(g);
    let (x, y) = match i {
        0..=8 => (8.0 + 18.0 * i as f32, 142.0),
        9..=35 => (8.0 + 18.0 * ((i - 9) % 9) as f32, 84.0 + 18.0 * ((i - 9) / 9) as f32),
        36..=39 => (8.0, 8.0 + 18.0 * (i - 36) as f32),
        40 => (77.0, 62.0),
        41..=44 => (98.0 + 18.0 * ((i - 41) % 2) as f32, 18.0 + 18.0 * ((i - 41) / 2) as f32),
        _ => (154.0, 28.0),
    };
    [p[0] + x * k, p[1] + y * k, 16.0 * k, 16.0 * k]
}

/// Vùng chạm 18x18 của ô túi i.
fn inv_slot_rect(g: &Gfx, i: usize) -> [f32; 4] {
    let r = inv_item_rect(g, i);
    let k = gs(g);
    [r[0] - k, r[1] - k, 18.0 * k, 18.0 * k]
}

fn rect_exit(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 92.0 * s, 36.0 * s, 80.0 * s, 40.0 * s]
}
fn rect_jump(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 88.0 * s, g.h as f32 - 200.0 * s, 76.0 * s, 76.0 * s]
}
fn rect_sneak(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 88.0 * s, g.h as f32 - 116.0 * s, 76.0 * s, 48.0 * s]
}
fn rect_fly(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [12.0 * s, g.h as f32 - 200.0 * s, 76.0 * s, 48.0 * s]
}
fn rect_sprint(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [12.0 * s, g.h as f32 - 144.0 * s, 76.0 * s, 48.0 * s]
}
fn inside(r: [f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

impl View {
    /// Cần context GL đang hiện hành.
    pub fn new(d: SceneData) -> Result<View, String> {
        let prog = build_program(VS, FS, &["a_pos", "a_uv", "a_col"])?;
        let sky = build_program(VS_SKY, FS_SKY, &[])?;
        unsafe {
            let c = |s: &str| CString::new(s).unwrap();
            let sky_view = glGetUniformLocation(sky, c("u_view").as_ptr());
            let sky_tf = glGetUniformLocation(sky, c("u_tf").as_ptr());
            let u_fog = glGetUniformLocation(prog, c("u_fog").as_ptr());
            let u_fogc = glGetUniformLocation(prog, c("u_fogc").as_ptr());
            let u_mvp = glGetUniformLocation(prog, c("u_mvp").as_ptr());
            let u_tex = glGetUniformLocation(prog, c("u_tex").as_ptr());
            let u_tint = glGetUniformLocation(prog, c("u_tint").as_ptr());
            let col = if d.stream.is_some() {
                let mut world = World::default();
                for (k, ch) in &d.col {
                    world.chunks.insert(*k, ch.clone());
                }
                Some(Collider { world, shapes: d.shapes.clone() })
            } else {
                None
            };
            let feet = [d.spawn[0] as f64, d.spawn[1] as f64 - 1.62, d.spawn[2] as f64];
            let mut gpu = HashMap::new();
            let mut nverts = 0;
            for c in &d.chunks {
                nverts += c.v.len();
                gpu.insert((c.cx, c.cz), upload(c));
            }
            let mut tex = 0;
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
            let n = crate::assets::ATLAS as i32;
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, n, n, 0, GL_RGBA, GL_UNSIGNED_BYTE, d.atlas.as_ptr() as *const _);
            for (p, v) in [
                (GL_TEXTURE_MIN_FILTER, GL_NEAREST),
                (GL_TEXTURE_MAG_FILTER, GL_NEAREST),
                (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
                (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
            ] {
                glTexParameteri(GL_TEXTURE_2D, p, v);
            }
            let inv_tex = d.items.inv.as_ref().map(|t| (make_tex(&t.rgba, t.w, t.h), t.w as f32, t.h as f32));
            let chest_tex = d.items.chest.as_ref().map(|t| (make_tex(&t.rgba, t.w, t.h), t.w as f32, t.h as f32));
            let hot_tex = d.items.hot.as_ref().map(|(t, r)| (make_tex(&t.rgba, t.w, t.h), t.w as f32, t.h as f32, *r));
            let sel_tex = d.items.sel.as_ref().map(|(t, r)| (make_tex(&t.rgba, t.w, t.h), t.w as f32, t.h as f32, *r));
            let body_gpu = upload_raw(&d.items.body_v, &d.items.body_i);
            let hud_tex: Vec<(u32, f32, f32)> = d.items.hud_tex.iter().map(|t| (make_tex(&t.rgba, t.w, t.h), t.w as f32, t.h as f32)).collect();
            let mut quad_vbo = 0;
            glGenBuffers(1, &mut quad_vbo);
            let items_gpu = upload_raw(&d.items.v, &d.items.i);
            let arm_gpu = upload_raw(&d.items.arm_v, &d.items.arm_i);
            let map_gpu = {
                let c = [255u8; 4];
                let v = [
                    V { p: [-0.5, -0.5, 0.0], uv: [0.0, 1.0], c },
                    V { p: [0.5, -0.5, 0.0], uv: [1.0, 1.0], c },
                    V { p: [0.5, 0.5, 0.0], uv: [1.0, 0.0], c },
                    V { p: [-0.5, 0.5, 0.0], uv: [0.0, 0.0], c },
                ];
                upload_raw(&v, &[0, 1, 2, 0, 2, 3])
            };
            let map_tex = make_tex(&[214u8, 186, 140, 255].repeat(128 * 128), 128, 128);
            let map_idx = include_str!("netitems.txt").lines().position(|n| n == "filled_map").map_or(-1, |p| p as i32);
            let skin_tex = make_tex(&d.items.skin.rgba, d.items.skin.w, d.items.skin.h);
            glBindTexture(GL_TEXTURE_2D, tex);
            let n_items = d.items.list.len();
            Ok(View {
                u_tint,
                inv_tex,
                chest_tex,
                win: None,
                win_pt: None,
                hot_tex,
                sel_tex,
                body_gpu,
                head_n: d.items.head_n,
                hud: d.items.hud,
                hud_tex,
                health: 20.0,
                fall: 0.0,
                hurt_t: 0.0,
                regen_t: 0.0,
                quad_vbo,
                items: d.items.list,
                items_gpu,
                arm_gpu,
                map_gpu,
                map_tex,
                map_idx,
                map_has: false,
                skin_tex,
                inv: Inv::new(n_items, d.items.wear, d.items.recipes),
                inv_open: false,
                ui_pt: (0.0, 0.0),
                ui_down: None,
                swing: None,
                lower: 1.0,
                shown: None,
                hand_b: 1.0,
                name_t: 0.0,
                last_held: (0, None),
                prog,
                sky,
                sky_view,
                sky_tf,
                u_fog,
                u_fogc,
                gpu,
                stream: d.stream,
                tex,
                u_mvp,
                u_tex,
                nverts,
                drawn_chunks: 0,
                drawn_verts: 0,
                pos: d.spawn,
                player: Player::new(feet, col.is_none()),
                col,
                prev: feet,
                prev_eye: 1.62,
                acc: 0.0,
                spawn: feet,
                jump_held: false,
                sneak_on: false,
                sprint_on: false,
                yaw: 0.0,
                pitch: -0.1,
                touches: Vec::new(),
                last: Instant::now(),
                fps_t: Instant::now(),
                frames: 0,
                fps: 0,
                exit: false,
                fov_deg: 70.0,
                frozen: false,
                net: d.net.clone(),
                kicked: None,
                net_state: d.net_state.clone(),
                chat_log: Vec::new(),
                chat_open: false,
                chat_text: String::new(),
                chat_tap: None,
                kb: crate::keyboard::Keyboard::new(),
                keys_in: Vec::new(),
                sent_sel: 0,
                key_names: d.names.clone(),
                attack_held: false,
                sent: 0,
                aim_dbg: String::new(),
                dig: None,
                place_req: false,
                others: HashMap::new(),
                tags: Vec::new(),
            })
        }
    }

    /// Vẽ một hình chữ nhật có ảnh: `dst` pixel màn hình, `src` điểm ảnh trong ảnh cỡ (tw, th). Cần chương trình + ma trận ortho đã đặt.
    unsafe fn quad(&self, tex: u32, dst: [f32; 4], src: [f32; 4], tw: f32, th: f32) {
        glBindTexture(GL_TEXTURE_2D, tex);
        let (u0, v0, u1, v1) = (src[0] / tw, src[1] / th, (src[0] + src[2]) / tw, (src[1] + src[3]) / th);
        let (x0, y0, x1, y1) = (dst[0], dst[1], dst[0] + dst[2], dst[1] + dst[3]);
        let c = [255u8; 4];
        let v = |x: f32, y: f32, u: f32, vv: f32| V { p: [x, y, 0.0], uv: [u, vv], c };
        let q = [v(x0, y0, u0, v0), v(x1, y0, u1, v0), v(x1, y1, u1, v1), v(x0, y0, u0, v0), v(x1, y1, u1, v1), v(x0, y1, u0, v1)];
        glBindBuffer(GL_ARRAY_BUFFER, self.quad_vbo);
        glBufferData(GL_ARRAY_BUFFER, (q.len() * std::mem::size_of::<V>()) as isize, q.as_ptr() as *const _, GL_STREAM_DRAW);
        let stride = std::mem::size_of::<V>() as i32;
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 3, GL_FLOAT, 0, stride, 0 as *const _);
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, stride, 12 as *const _);
        glEnableVertexAttribArray(2);
        glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, 1, stride, 20 as *const _);
        glDrawArrays(GL_TRIANGLES, 0, 6);
    }

    /// Bố cục cửa sổ vật chứa đang mở.
    fn win_layout(&self, g: &Gfx) -> Option<crate::winui::Layout> {
        let w = self.win.as_ref()?;
        let nc = crate::winui::container_slots(w.slots.len());
        Some(crate::winui::layout(g.w as f32, g.h as f32, crate::winui::rows_for(w.kind, nc), nc))
    }

    /// Đóng cửa sổ vật chứa và báo server.
    fn win_close(&mut self) {
        self.touches.clear();
        if let Some(w) = self.win.take() {
            if let Some(tx) = &self.net {
                let _ = tx.send(crate::netplay::NetOut::CloseWin(w.id));
            }
        }
        if let Some(st) = &self.net_state {
            let mut g = st.lock().unwrap();
            g.win = None;
            g.win_dirty = false;
        }
    }

    /// Nhả ngón trong cửa sổ vật chứa: bấm ô vật chứa, nút Đóng, hoặc chạm ra ngoài khung để đóng.
    fn win_tap(&mut self, g: &Gfx, x: f32, y: f32) {
        if inside(rect_exit(g), x, y) {
            self.exit = true;
            return;
        }
        let Some(l) = self.win_layout(g) else { return };
        if inside(rect_bag(g), x, y) || !inside(l.panel, x, y) {
            self.win_close();
            return;
        }
        let Some(w) = &self.win else { return };
        let nc = crate::winui::container_slots(w.slots.len());
        if let Some(i) = crate::winui::hit(&l, x, y) {
            // chỉ ô vật chứa có vật mới gửi click (túi người chơi chỉ để xem)
            if i < nc && w.slots.get(i).map_or(false, |s| s.is_some()) {
                if let Some(tx) = &self.net {
                    let _ = tx.send(crate::netplay::NetOut::Click { win: w.id, state: w.state_id, slot: i as i16, button: 0, mode: 0 });
                }
            }
        }
    }

    pub fn take_kick(&mut self) -> Option<String> {
        self.kicked.take()
    }

    pub fn is_net(&self) -> bool {
        self.net.is_some()
    }

    pub fn set_frozen(&mut self, f: bool) {
        self.frozen = f;
        if f {
            self.touches.clear();
        }
    }

    pub fn touch(&mut self, id: i32, phase: Phase, x: f32, y: f32, g: &Gfx) {
        if self.frozen {
            return;
        }
        if self.win.is_some() {
            match phase {
                Phase::Up => {
                    self.win_pt = None;
                    self.win_tap(g, x, y);
                }
                _ => self.win_pt = Some((x, y)),
            }
            return;
        }
        if self.chat_open {
            // màn chat: chỉ nhận chạm cho nút/bàn phím (xử lý lúc vẽ)
            if phase == Phase::Up {
                self.chat_tap = Some((x, y));
            }
            return;
        }
        match phase {
            Phase::Down => {
                let hot = if self.inv_open { None } else { (0..9).find(|&i| inside(hot_slot_rect(g, i), x, y)) };
                let role = if inside(rect_exit(g), x, y) {
                    Role::Exit
                } else if inside(rect_bag(g), x, y) {
                    Role::Bag
                } else if self.net.is_some() && !self.inv_open && inside(rect_chat(g), x, y) {
                    Role::Chat
                } else if self.inv_open {
                    Role::Ui
                } else if let Some(i) = hot {
                    Role::Hot(i)
                } else if self.net.is_some() && inside(rect_attack(g), x, y) {
                    Role::Attack
                } else if self.net.is_some() && inside(rect_place(g), x, y) {
                    Role::Place
                } else if inside(rect_jump(g), x, y) {
                    Role::Jump
                } else if inside(rect_sneak(g), x, y) {
                    Role::Sneak
                } else if inside(rect_fly(g), x, y) {
                    Role::Fly
                } else if inside(rect_sprint(g), x, y) {
                    Role::Sprint
                } else if x < g.w as f32 * 0.5 {
                    Role::Move
                } else {
                    Role::Look
                };
                self.touches.retain(|t| t.id != id);
                if role == Role::Ui {
                    self.ui_pt = (x, y);
                    // chạm vào ô khi tay đang trống: nhấc luôn lên để kéo thả
                    self.ui_down = (0..SLOTS).find(|&i| inside(inv_slot_rect(g, i), x, y)).map(|i| {
                        let pick = self.net.is_none() && self.inv.cursor.is_none() && self.inv.slots[i].is_some();
                        if pick {
                            self.inv.click(i);
                        }
                        (i, pick)
                    });
                }
                self.touches.push(Touch { id, role, sx: x, sy: y, x, y, t0: Instant::now(), moved: 0.0 });
            }
            Phase::Move => {
                let sens = 0.0035; // rad / pixel
                if let Some(t) = self.touches.iter_mut().find(|t| t.id == id) {
                    if t.role == Role::Look {
                        self.yaw -= (x - t.x) * sens;
                        self.pitch = (self.pitch - (y - t.y) * sens).clamp(-1.5, 1.5);
                        t.moved += (x - t.x).abs() + (y - t.y).abs();
                    }
                    if t.role == Role::Ui {
                        self.ui_pt = (x, y);
                    }
                    t.x = x;
                    t.y = y;
                }
            }
            Phase::Up => {
                if let Some(i) = self.touches.iter().position(|t| t.id == id) {
                    let t = self.touches.remove(i);
                    // nút bật/tắt: kích hoạt khi nhả ngón trong chính nút đó
                    match t.role {
                        Role::Exit if inside(rect_exit(g), x, y) => self.exit = true,
                        Role::Bag if inside(rect_bag(g), x, y) => {
                            self.inv_open = !self.inv_open;
                            if self.inv_open {
                                self.touches.clear();
                                self.ui_down = None;
                                self.ui_pt = (g.w as f32 / 2.0, g.h as f32 / 2.0);
                            } else {
                                self.inv.stash();
                            }
                        }
                        Role::Place if inside(rect_place(g), x, y) => self.place_req = true,
                        Role::Chat if inside(rect_chat(g), x, y) => {
                            self.chat_open = true;
                            self.chat_tap = None;
                            self.kb.reset();
                            self.touches.clear();
                        }
                        Role::Hot(i) if inside(hot_slot_rect(g, i), x, y) => self.inv.sel = i,
                        Role::Look if t.moved < 14.0 * g.scale && t.t0.elapsed().as_secs_f32() < 0.3 && self.swing.is_none() => self.swing = Some(0.0),
                        Role::Ui => {
                            let up = (0..SLOTS).find(|&i| inside(inv_slot_rect(g, i), x, y));
                            // nhiều người chơi: túi chỉ để xem (server giữ dữ liệu thật)
                            let up = if self.net.is_some() { None } else { up };
                            match (self.ui_down.take(), up) {
                                // đang cầm vật: thả vào ô dưới ngón tay
                                (Some((_, false)), Some(u)) => self.inv.click(u),
                                // kéo từ ô này sang ô khác: thả
                                (Some((d, true)), Some(u)) if d != u => self.inv.click(u),
                                // thả ra ngoài khung túi khi đang cầm: vứt bỏ
                                (_, None) if self.net.is_none() && self.inv.cursor.is_some() => {
                                    if !inside(inv_panel(g), x, y) {
                                        self.inv.cursor = None;
                                    }
                                }
                                _ => {}
                            }
                        }
                        Role::Sneak if inside(rect_sneak(g), x, y) => self.sneak_on = !self.sneak_on,
                        Role::Sprint if inside(rect_sprint(g), x, y) => self.sprint_on = !self.sprint_on,
                        Role::Fly if inside(rect_fly(g), x, y) && self.col.is_some() => {
                            self.player.fly = !self.player.fly;
                            if self.player.fly {
                                self.player.vel[1] = 0.0;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Chat đang mở -> cần bàn phím hệ thống.
    pub fn wants_text(&self) -> bool {
        // bàn phím hệ thống bị đơ nên tạm tắt; dùng bàn phím tự vẽ
        false
    }

    pub fn key_input(&mut self, k: KeyIn) {
        if self.chat_open {
            self.keys_in.push(k);
        }
    }

    /// Tia ngắm từ mắt theo hướng nhìn, tới 5 khối; trả về khối trúng, mặt và điểm trúng.
    fn aim(&self) -> Option<Aim> {
        let col = self.col.as_ref()?;
        let (sy, cy, sp, cp) = (self.yaw.sin(), self.yaw.cos(), self.pitch.sin(), self.pitch.cos());
        let d = [-sy * cp, sp, -cy * cp];
        let o = self.pos;
        let mut cell = [o[0].floor() as i32, o[1].floor() as i32, o[2].floor() as i32];
        let mut step = [0i32; 3];
        let mut tmax = [f32::MAX; 3];
        let mut tdel = [f32::MAX; 3];
        for k in 0..3 {
            if d[k] > 1e-6 {
                step[k] = 1;
                tdel[k] = 1.0 / d[k];
                tmax[k] = (cell[k] as f32 + 1.0 - o[k]) / d[k];
            } else if d[k] < -1e-6 {
                step[k] = -1;
                tdel[k] = -1.0 / d[k];
                tmax[k] = (o[k] - cell[k] as f32) / -d[k];
            }
        }
        let mut t = 0.0;
        let mut face = 0u8;
        while t <= 5.0 {
            let id = col.world.get(cell[0], cell[1], cell[2]);
            if id != 0 && id != crate::world::UNKNOWN {
                if let Some(sh) = col.shapes.get(id as usize) {
                    if sh.fluid == 0 && !sh.boxes.is_empty() {
                        let p = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
                        let cur = [(p[0] - cell[0] as f32).clamp(0.0, 1.0), (p[1] - cell[1] as f32).clamp(0.0, 1.0), (p[2] - cell[2] as f32).clamp(0.0, 1.0)];
                        return Some(Aim { pos: cell, face, cur });
                    }
                }
            }
            let k = if tmax[0] < tmax[1] && tmax[0] < tmax[2] { 0 } else if tmax[1] < tmax[2] { 1 } else { 2 };
            if step[k] == 0 {
                return None;
            }
            t = tmax[k];
            tmax[k] += tdel[k];
            cell[k] += step[k];
            // mặt của khối mới bị tia đi vào (hướng 0 xuống,1 lên,2 bắc -z,3 nam +z,4 tây -x,5 đông +x)
            face = match (k, step[k]) {
                (0, 1) => 4,
                (0, _) => 5,
                (1, 1) => 0,
                (1, _) => 1,
                (2, 1) => 2,
                _ => 3,
            };
        }
        None
    }

    /// Đánh (giữ để đào) và Đặt gửi lên server.
    fn net_actions(&mut self, now: Instant) {
        let Some(tx) = self.net.clone() else { return };
        if (self.place_req || self.attack_held) && self.swing.is_none() {
            self.swing = Some(0.0);
        }
        let aim = self.aim();
        self.aim_dbg = match &aim {
            Some(a) => {
                let id = self.col.as_ref().map_or(0, |c| c.world.get(a.pos[0], a.pos[1], a.pos[2]));
                format!("{} {},{},{} mặt {}", self.state_name(id), a.pos[0], a.pos[1], a.pos[2], a.face)
            }
            None => "không trúng khối nào".to_string(),
        };
        if self.place_req {
            self.sent += 1;
            self.place_req = false;
            let _ = tx.send(crate::netplay::NetOut::Swing);
            match &aim {
                Some(a) => {
                    let _ = tx.send(crate::netplay::NetOut::Place { pos: a.pos, face: a.face, cur: a.cur });
                }
                None => {
                    let _ = tx.send(crate::netplay::NetOut::Use { yaw: 180.0 - self.yaw.to_degrees(), pitch: -self.pitch.to_degrees() });
                }
            }
        }
        if !self.attack_held {
            if let Some(d) = self.dig.take() {
                let _ = tx.send(crate::netplay::NetOut::Dig { status: 1, pos: d.pos, face: 1 });
            }
            return;
        }
        let Some(a) = aim else {
            if let Some(d) = self.dig.take() {
                let _ = tx.send(crate::netplay::NetOut::Dig { status: 1, pos: d.pos, face: 1 });
            }
            return;
        };
        // đổi sang khối khác thì hủy cái cũ
        if self.dig.as_ref().map_or(false, |d| d.pos != a.pos) {
            let d = self.dig.take().unwrap();
            let _ = tx.send(crate::netplay::NetOut::Dig { status: 1, pos: d.pos, face: 1 });
        }
        let id = self.col.as_ref().map_or(0, |c| c.world.get(a.pos[0], a.pos[1], a.pos[2]));
        if self.dig.is_none() {
            let key = self.state_name(id);
            let (h, tool) = hardness(&key);
            let need = if h < 0.0 { 1e9 } else { h * if tool { 5.0 } else { 1.5 } + 0.05 };
            self.sent += 1;
            let _ = tx.send(crate::netplay::NetOut::Swing);
            let _ = tx.send(crate::netplay::NetOut::Dig { status: 0, pos: a.pos, face: a.face });
            self.dig = Some(Dig { pos: a.pos, t0: now, need, finished: None });
            return;
        }
        let d = self.dig.as_mut().unwrap();
        if d.finished.is_none() && now.duration_since(d.t0).as_secs_f32() >= d.need {
            let _ = tx.send(crate::netplay::NetOut::Dig { status: 2, pos: d.pos, face: a.face });
            d.finished = Some(now);
        } else if let Some(f) = d.finished {
            // khối vẫn còn sau 0.4 giây: bắt đầu đào lại
            if now.duration_since(f).as_secs_f32() > 0.4 {
                let _ = tx.send(crate::netplay::NetOut::Dig { status: 0, pos: d.pos, face: a.face });
                d.t0 = now;
                d.finished = None;
            }
        }
    }

    /// Tên block (không thuộc tính) của id toàn cục, để tra độ cứng.
    fn state_name(&self, id: u16) -> String {
        self.key_names.get(id as usize).cloned().unwrap_or_default()
    }

    /// Lượt cuối (2D): tin nhắn, nút chat, và màn chat có bàn phím.
    fn chat_pass(&mut self, g: &mut Gfx) {
        if self.net.is_none() || self.inv_open || self.win.is_some() {
            return;
        }
        let s = g.scale;
        let (w, h) = (g.w as f32, g.h as f32);
        let size = 13.0 * s;
        let lh = size * 1.25;
        g.begin();
        if !self.chat_open {
            // dòng dò lỗi dưới chấm ngắm: khối đang ngắm, số lần gửi, số xác nhận của server
            let ack = self.net_state.as_ref().map_or(0, |st| st.lock().unwrap().ack);
            let (px, pz) = (self.player.pos[0].floor() as i32, self.player.pos[2].floor() as i32);
            let ld = self.col.as_ref().map_or(0, |c| c.world.chunks.len());
            let here = self.col.as_ref().map_or(true, |c| c.loaded(px, pz));
            let dbg = format!("{} | gửi {} | xn {} | chunk {} {} | y {:.1}", self.aim_dbg, self.sent, ack, ld, if here { "ok" } else { "CHƯA NẠP" }, self.player.pos[1]);
            let (tpn, tpd) = self.net_state.as_ref().map_or((0, String::new()), |st| {
                let g = st.lock().unwrap();
                (g.tp_n, g.tp_dbg.clone())
            });
            static LAST_TP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if tpn != LAST_TP.swap(tpn, std::sync::atomic::Ordering::Relaxed) {
                if let Some(c) = &self.col {
                    let p = self.player.pos;
                    let (bx, by, bz) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
                    let mut near = String::new();
                    for dy in -1..=2 {
                        for dz in -1..=1 {
                            for dx in -1..=1 {
                                let id = c.world.get(bx + dx, by + dy, bz + dz);
                                if id != 0 {
                                    near.push_str(&format!("[{dx},{dy},{dz}]{}{} ", self.state_name(id), c.shapes.get(id as usize).map_or(String::new(), |s| format!("/{}hop", s.boxes.len()))));
                                }
                            }
                        }
                    }
                    crate::netplay::trace(format!("CTX view pos=({:.3},{:.3},{:.3}) vel=({:.3},{:.3},{:.3}) dat={} khối quanh: {}", p[0], p[1], p[2], self.player.vel[0], self.player.vel[1], self.player.vel[2], self.player.on_ground, near));
                    crate::netplay::trace_flush();
                }
            }
            let tw2 = g.text_width(&tpd, 11.0 * s);
            g.text(w / 2.0 - tw2 / 2.0, h / 2.0 + 28.0 * s, 11.0 * s, 0xffff80e0, &tpd);
            let tw = g.text_width(&dbg, 11.0 * s);
            g.text(w / 2.0 - tw / 2.0, h / 2.0 + 14.0 * s, 11.0 * s, 0xffffffd0, &dbg);
            // chấm ngắm giữa màn hình
            let (cx, cy, t) = (w / 2.0, h / 2.0, 2.0 * s);
            g.rect(cx - 8.0 * s, cy - t / 2.0, 16.0 * s, t, 0xffffffc0);
            g.rect(cx - t / 2.0, cy - 8.0 * s, t, 16.0 * s, 0xffffffc0);
            // tên người chơi gần đó
            let ts = 12.0 * s;
            for (x, y, name) in &self.tags {
                let tw = g.text_width(name, ts);
                g.rect(x - tw / 2.0 - 4.0 * s, y - ts * 1.3, tw + 8.0 * s, ts * 1.4, 0x00000080);
                g.text(x - tw / 2.0, y - ts * 1.25, ts, 0xffffffff, name);
            }
            // tin gần đây ở góc trên-trái, mờ dần sau 9 giây
            let maxw = w * 0.5;
            let mut shown: Vec<(String, u32)> = Vec::new();
            for (l, t) in self.chat_log.iter().rev().take(8) {
                let age = t.elapsed().as_secs_f32();
                if age > 10.0 {
                    break;
                }
                let a = if age < 8.0 { 255 } else { (255.0 * (10.0 - age) / 2.0) as u32 };
                for ln in wrap_legacy(g, l, size, maxw).into_iter().rev() {
                    shown.push((ln, a));
                }
            }
            shown.reverse();
            let n = shown.len().min(10);
            let y0 = 108.0 * s;
            for (j, (ln, a)) in shown.iter().skip(shown.len() - n).enumerate() {
                let y = y0 + j as f32 * lh;
                g.rect(10.0 * s, y - 1.0, maxw + 4.0 * s, lh, 0x00000000 | (a * 100 / 255));
                draw_legacy(g, 12.0 * s, y, size, ln, *a);
            }
            let r = rect_chat(g);
            g.rect(r[0], r[1], r[2], r[3], 0x00000090);
            let tw = g.text_width("Chat", 14.0 * s);
            g.text(r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - 18.0 * s) / 2.0, 14.0 * s, 0xffffffff, "Chat");
            g.end(None);
            return;
        }
        // màn chat: nền tối, lịch sử, ô nhập, bàn phím chiếm nửa dưới
        let tap = self.chat_tap.take();
        g.rect(0.0, 0.0, w, h, 0x000000a0);
        let kb_h = h * 0.52;
        let kb_area = crate::ui::Rect::new(0.0, h - kb_h, w, kb_h);
        let bar_h = 34.0 * s;
        let bar_y = h - kb_h - bar_h - 6.0 * s;
        let maxw = w - 24.0 * s;
        let mut lines: Vec<String> = Vec::new();
        for (l, _) in self.chat_log.iter().rev().take(30) {
            for ln in wrap_legacy(g, l, size, maxw).into_iter().rev() {
                lines.push(ln);
            }
            if lines.len() > 40 {
                break;
            }
        }
        let rows = (((bar_y - 12.0 * s) / lh) as usize).min(lines.len());
        for j in 0..rows {
            let y = bar_y - 8.0 * s - (j + 1) as f32 * lh;
            draw_legacy(g, 12.0 * s, y, size, &lines[j], 255);
        }
        g.rect(8.0 * s, bar_y, w - 16.0 * s, bar_h, 0x000000d0);
        g.rect(8.0 * s, bar_y, w - 16.0 * s, 2.0 * s, 0xffffffa0);
        let shown_text = format!("{}_", self.chat_text);
        let mut t = shown_text.as_str();
        while g.text_width(t, size) > w - 40.0 * s && !t.is_empty() {
            t = &t[t.chars().next().map_or(0, |c| c.len_utf8())..];
        }
        g.text(16.0 * s, bar_y + (bar_h - size * 1.2) / 2.0, size, 0xffffffff, t);
        let mut send = false;
        let mut close = false;
        match self.kb.draw(g, kb_area, tap) {
            Some(crate::keyboard::Kb::Ch(c)) => {
                if self.chat_text.chars().count() < 256 {
                    self.chat_text.push(c);
                }
            }
            Some(crate::keyboard::Kb::Back) => {
                self.chat_text.pop();
            }
            Some(crate::keyboard::Kb::Done) => send = true,
            Some(crate::keyboard::Kb::Cancel) => close = true,
            _ => {}
        }
        if send {
            let t = self.chat_text.trim().to_string();
            if !t.is_empty() {
                if let Some(n) = &self.net {
                    let _ = n.send(crate::netplay::NetOut::Chat(t));
                }
            }
            self.chat_text.clear();
            close = true;
        }
        if close {
            self.chat_open = false;
            self.chat_tap = None;
        }
        g.end(None);
    }

    /// Vẽ một khung hình. Trả về true khi người dùng bấm Thoát.
    pub fn frame(&mut self, g: &mut Gfx) -> bool {
        let now = Instant::now();
        let dt = if self.frozen { 0.0 } else { (now - self.last).as_secs_f32().min(0.1) };
        self.last = now;
        let s = g.scale;
        if let Some(st) = &self.net_state {
            let (new, raw) = {
                let mut g = st.lock().unwrap();
                if g.inv_dirty {
                    g.inv_dirty = false;
                    for i in 0..SLOTS {
                        self.inv.slots[i] = g.inv.get(i).copied().flatten().map(|(id, n)| crate::inv::Stack { item: id as u16, n });
                    }
                }
                if g.win_dirty {
                    g.win_dirty = false;
                    let nw = g.win.clone();
                    if self.win.is_none() && nw.is_some() {
                        // server vừa mở cửa sổ: bỏ mọi thao tác đang giữ
                        self.touches.clear();
                        self.inv_open = false;
                        self.chat_open = false;
                        self.ui_down = None;
                        self.attack_held = false;
                        self.win_pt = None;
                    }
                    self.win = nw;
                }
                if let Some(h) = g.held.take() {
                    self.inv.sel = h as usize;
                    self.sent_sel = h as usize;
                }
                let raw: Vec<(i32, crate::netplay::Pl, String)> = g.players.iter().map(|(e, p)| (*e, p.clone(), g.names.get(&p.uuid).cloned().unwrap_or_default())).collect();
                (std::mem::take(&mut g.chat), raw)
            };
            let k = (dt * 10.0).min(1.0);
            self.others.retain(|e, _| raw.iter().any(|r| r.0 == *e));
            for (e, p, name) in raw {
                let o = self.others.entry(e).or_insert_with(|| Other { pos: [p.pos[0] as f32, p.pos[1] as f32, p.pos[2] as f32], tgt: p.pos, yaw: p.yaw, pitch: p.pitch, head: p.head, name: String::new() });
                o.tgt = p.pos;
                for j in 0..3 {
                    o.pos[j] += (p.pos[j] as f32 - o.pos[j]) * k;
                }
                o.yaw = lerp_ang(o.yaw, p.yaw, k);
                o.head = lerp_ang(o.head, p.head, k);
                o.pitch += (p.pitch - o.pitch) * k;
                o.name = name;
            }
            if self.inv.sel != self.sent_sel {
                self.sent_sel = self.inv.sel;
                if let Some(n) = &self.net {
                    let _ = n.send(crate::netplay::NetOut::Held(self.inv.sel as u8));
                }
            }
            for l in new {
                self.chat_log.push((l, now));
            }
            let n = self.chat_log.len();
            if n > 100 {
                self.chat_log.drain(..n - 100);
            }
        }

        // tay / khối cầm: hạ xuống rồi nâng lên khi đổi vật; vung khi chạm
        let want = self.inv.held();
        if self.shown != want {
            self.lower += dt / 0.12;
            if self.lower >= 1.0 {
                self.lower = 1.0;
                self.shown = want;
            }
        } else {
            self.lower = (self.lower - dt / 0.12).max(0.0);
        }
        if let Some(t) = self.swing.as_mut() {
            *t += dt;
            if *t >= SWING_T {
                self.swing = None;
            }
        }
        if self.last_held != (self.inv.sel, want) {
            self.last_held = (self.inv.sel, want);
            self.name_t = 2.0;
        }
        self.name_t = (self.name_t - dt).max(0.0);
        self.hurt_t = (self.hurt_t - dt).max(0.0);
        if self.health < 20.0 {
            self.regen_t += dt;
            if self.regen_t >= 1.0 {
                self.regen_t = 0.0;
                self.health = (self.health + 1.0).min(20.0);
            }
        }

        // stream chunk: báo vị trí cho luồng nền, nhận mesh mới (tối đa 4 chunk/khung để không giật)
        if let Some(st) = &self.stream {
            st.pos.store(pack((self.pos[0].floor() as i32) >> 4, (self.pos[2].floor() as i32) >> 4), Relaxed);
        }
        let mut uploads = 0;
        while uploads < 4 {
            let m = match &self.stream {
                Some(st) => st.rx.try_recv().ok(),
                None => None,
            };
            let Some(m) = m else { break };
            match m {
                Msg::Add(c) => {
                    uploads += 1;
                    let g = unsafe { upload(&c) };
                    self.nverts += c.v.len();
                    if let Some(old) = self.gpu.insert((c.cx, c.cz), g) {
                        self.nverts -= old.nverts;
                        unsafe { free(&old) };
                    }
                }
                Msg::Drop(x, z) => {
                    if let Some(old) = self.gpu.remove(&(x, z)) {
                        self.nverts -= old.nverts;
                        unsafe { free(&old) };
                    }
                }
                Msg::Names(n) => self.key_names.extend(n),
                Msg::Shapes(sh) => {
                    if let Some(c) = self.col.as_mut() {
                        c.shapes.extend(sh);
                    }
                }
                Msg::Col(x, z, ch) => {
                    if let Some(c) = self.col.as_mut() {
                        c.world.chunks.insert((x, z), ch);
                    }
                }
                Msg::ColDrop(x, z) => {
                    if let Some(c) = self.col.as_mut() {
                        c.world.chunks.remove(&(x, z));
                    }
                }
                Msg::Teleport(p, yaw, pitch) => {
                    self.player.pos = p;
                    self.player.vel = [0.0; 3];
                    self.prev = p;
                    self.yaw = std::f32::consts::PI - yaw.to_radians();
                    self.pitch = (-pitch.to_radians()).clamp(-1.5, 1.5);
                    self.fall = 0.0;
                }
                Msg::Kick(m) => self.kicked = Some(m),
            }
        }

        // đầu vào: cần trái = (dịch chuyển ngón tay so với lúc chạm) / 70dp; kéo lên = tiến
        let (mut sx, mut sy) = (0.0f32, 0.0f32);
        let mut attack = false;
        self.jump_held = false;
        for t in &self.touches {
            match t.role {
                Role::Move => {
                    sx = ((t.x - t.sx) / (70.0 * s)).clamp(-1.0, 1.0);
                    sy = ((t.y - t.sy) / (70.0 * s)).clamp(-1.0, 1.0);
                }
                Role::Jump => self.jump_held = true,
                Role::Attack => attack = true,
                _ => {}
            }
        }
        self.attack_held = attack;
        self.net_actions(now);
        // bàn phím Java chỉ có 0/±1 cho mỗi hướng; anticheat đối chiếu đúng các giá trị rời rạc đó nên cần lượng tử hóa cần analog
        let q = |v: f32| if v > 0.25 { 1.0f64 } else if v < -0.25 { -1.0 } else { 0.0 };
        let inp = Input { fwd: q(-sy), strafe: q(sx), jump: self.jump_held, sneak: self.sneak_on, sprint: self.sprint_on };

        // vật lý theo tick cố định 20/giây, camera nội suy giữa hai tick
        if self.col.as_ref().map_or(false, |c| c.loaded(self.player.pos[0].floor() as i32, self.player.pos[2].floor() as i32)) || self.col.is_none() {
            self.acc += dt;
            let mut n = 0;
            while self.acc >= TICK && n < 5 {
                self.acc -= TICK;
                n += 1;
                self.prev = self.player.pos;
                self.prev_eye = self.player.eye_height();
                self.player.tick(&inp, self.yaw as f64, self.pitch as f64, self.col.as_ref());
                if let Some(n) = &self.net {
                    let b = (inp.fwd > 0.1) as u8 | (((inp.fwd < -0.1) as u8) << 1) | (((inp.strafe < -0.1) as u8) << 2) | (((inp.strafe > 0.1) as u8) << 3) | ((inp.jump as u8) << 4) | ((self.player.sneaking as u8) << 5) | ((self.player.sprinting as u8) << 6);
                    let _ = n.send(crate::netplay::NetOut::Input(b));
                    let _ = n.send(crate::netplay::NetOut::TickEnd);
                }
                if let Some(n) = &self.net {
                    let p = self.player.pos;
                    let _ = n.send(crate::netplay::NetOut::Pos { x: p[0], y: p[1], z: p[2], yaw: 180.0 - self.yaw.to_degrees(), pitch: -self.pitch.to_degrees(), ground: self.player.on_ground, hcol: self.player.hcol });
                }
                // rơi: sát thương = quãng rơi - 3 (làm tròn lên), nước và bay thì không
                if self.col.is_some() && !self.player.fly && self.net.is_none() {
                    if self.player.in_water {
                        self.fall = 0.0;
                    } else if self.player.on_ground {
                        if self.fall > 3.0 {
                            self.health -= (self.fall - 3.0).ceil();
                            self.hurt_t = 0.4;
                        }
                        self.fall = 0.0;
                    } else if self.player.pos[1] < self.prev[1] {
                        self.fall += (self.prev[1] - self.player.pos[1]) as f32;
                    }
                } else {
                    self.fall = 0.0;
                }
                if self.health <= 0.0 {
                    self.player.pos = self.spawn;
                    self.player.vel = [0.0; 3];
                    self.health = 20.0;
                    self.fall = 0.0;
                }
                if self.player.pos[1] < -100.0 && self.net.is_none() {
                    self.player.pos = self.spawn;
                    self.player.vel = [0.0; 3];
                }
            }
            if n == 5 {
                self.acc = 0.0;
            }
        }
        let a = (self.acc / TICK).clamp(0.0, 1.0) as f64;
        let cam = |i: usize| self.prev[i] + (self.player.pos[i] - self.prev[i]) * a;
        let eye = self.prev_eye + (self.player.eye_height() - self.prev_eye) * a;
        self.pos = [cam(0) as f32, (cam(1) + eye) as f32, cam(2) as f32];

        // độ sáng của tay theo ánh sáng thế giới tại mắt (làm mượt)
        let target_b = self.col.as_ref().map_or(1.0, |c| bright(c.world.light(self.pos[0].floor() as i32, self.pos[1].floor() as i32, self.pos[2].floor() as i32)));
        self.hand_b += (target_b - self.hand_b) * (dt * 8.0).min(1.0);

        // mắt đang ở trong nước / dung nham: sương mù dày màu xanh / cam, không thấy trời
        let medium = self.col.as_ref().map_or(0, |c| c.fluid_at(self.pos[0].floor() as i32, self.pos[1].floor() as i32, self.pos[2].floor() as i32));
        let (fog_c, fog_s, fog_e) = match medium {
            1 => ([0.10f32, 0.22, 0.50], 0.0f32, 22.0f32),
            2 => ([0.55, 0.10, 0.02], 0.0, 3.0),
            _ => ([0.53, 0.81, 0.92], FOG_START, FOG_END),
        };
        let aspect = g.w as f32 / g.h as f32;
        let mvp = math::mul(&math::perspective(self.fov_deg.to_radians(), aspect, 0.05, 300.0), &math::view(self.pos, self.yaw, self.pitch));
        unsafe {
            glViewport(0, 0, g.w, g.h);
            glDisable(GL_SCISSOR_TEST);
            glClearColor(fog_c[0], fog_c[1], fog_c[2], 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            // bầu trời: tam giác phủ màn hình, hướng nhìn tính từ ma trận view
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_CULL_FACE);
            glDisable(GL_BLEND);
            for i in 0..3 {
                glDisableVertexAttribArray(i);
            }
            if medium == 0 {
                glUseProgram(self.sky);
                let vm = math::view(self.pos, self.yaw, self.pitch);
                glUniformMatrix4fv(self.sky_view, 1, 0, vm.as_ptr());
                let tf = (1.2f32 / 2.0).tan();
                glUniform2f(self.sky_tf, tf * aspect, tf);
                glDrawArrays(GL_TRIANGLES, 0, 3);
            }
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glDisable(GL_BLEND);
            glEnable(GL_CULL_FACE);
            glUseProgram(self.prog);
            glUniform2f(self.u_fog, fog_s, fog_e);
            glUniform3f(self.u_fogc, fog_c[0], fog_c[1], fog_c[2]);
            glUniform3f(self.u_tint, 1.0, 1.0, 1.0);
            glUniformMatrix4fv(self.u_mvp, 1, 0, mvp.as_ptr());
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, self.tex);
            glUniform1i(self.u_tex, 0);
            let stride = std::mem::size_of::<V>() as i32;
            let fr = math::frustum(&mvp);
            let vis: Vec<&Gpu> = self.gpu.values().filter(|c| math::aabb_visible(&fr, &c.bb)).collect();
            let bind = |c: &Gpu| {
                glBindBuffer(GL_ARRAY_BUFFER, c.vbo);
                glEnableVertexAttribArray(0);
                glVertexAttribPointer(0, 3, GL_FLOAT, 0, stride, 0 as *const _);
                glEnableVertexAttribArray(1);
                glVertexAttribPointer(1, 2, GL_FLOAT, 0, stride, 12 as *const _);
                glEnableVertexAttribArray(2);
                glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, 1, stride, 20 as *const _);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, c.ebo);
            };
            // lượt 1: khối đặc
            for c in &vis {
                bind(c);
                glDrawElements(GL_TRIANGLES, c.count, GL_UNSIGNED_INT, 0 as *const _);
            }
            // lượt 2: nước nửa trong suốt (không ghi độ sâu)
            if vis.iter().any(|c| c.cw > 0) {
                glEnable(GL_BLEND);
                glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
                glDepthMask(0);
                for c in &vis {
                    if c.cw > 0 {
                        bind(c);
                        glDrawElements(GL_TRIANGLES, c.cw, GL_UNSIGNED_INT, (c.count as usize * 4) as *const _);
                    }
                }
                glDepthMask(1);
                glDisable(GL_BLEND);
            }
            self.drawn_chunks = vis.len();
            self.drawn_verts = vis.iter().map(|c| c.nverts).sum();
            // người chơi khác: thân Steve đứng yên, đầu quay theo hướng nhìn
            self.tags.clear();
            if !self.others.is_empty() {
                glDisable(GL_CULL_FACE);
                glBindTexture(GL_TEXTURE_2D, self.skin_tex);
                bind(&self.body_gpu);
                let hn = self.head_n;
                let sc = 0.9375 / 16.0;
                for o in self.others.values() {
                    let base = math::chain(&[
                        math::translate(o.pos[0], o.pos[1], o.pos[2]),
                        math::rot_y((180.0 - o.yaw).to_radians()),
                        math::scale(-sc, -sc, sc),
                        math::translate(0.0, -24.0, 0.0),
                    ]);
                    let mb = math::mul(&mvp, &base);
                    let mh = math::chain(&[mvp, base, math::rot_y((o.head - o.yaw).to_radians()), math::rot_x(o.pitch.to_radians())]);
                    glUniformMatrix4fv(self.u_mvp, 1, 0, mh.as_ptr());
                    glDrawElements(GL_TRIANGLES, hn, GL_UNSIGNED_INT, 0 as *const _);
                    glUniformMatrix4fv(self.u_mvp, 1, 0, mb.as_ptr());
                    glDrawElements(GL_TRIANGLES, self.body_gpu.count - hn, GL_UNSIGNED_INT, (hn as usize * 4) as *const _);
                    if !o.name.is_empty() {
                        let (x, y, z) = (o.pos[0], o.pos[1] + 2.1, o.pos[2]);
                        let cw = mvp[3] * x + mvp[7] * y + mvp[11] * z + mvp[15];
                        if cw > 0.1 && cw < 64.0 {
                            let cx = (mvp[0] * x + mvp[4] * y + mvp[8] * z + mvp[12]) / cw;
                            let cy = (mvp[1] * x + mvp[5] * y + mvp[9] * z + mvp[13]) / cw;
                            if cx.abs() < 1.2 && cy.abs() < 1.2 {
                                self.tags.push(((cx * 0.5 + 0.5) * g.w as f32, (0.5 - cy * 0.5) * g.h as f32, o.name.clone()));
                            }
                        }
                    }
                }
                glBindTexture(GL_TEXTURE_2D, self.tex);
                glEnable(GL_CULL_FACE);
                glUniformMatrix4fv(self.u_mvp, 1, 0, mvp.as_ptr());
            }

            // tay Steve / khối cầm: vẽ sau cùng, xoá độ sâu để không bị khối thế giới che
            if !self.inv_open && self.win.is_none() {
                glClear(GL_DEPTH_BUFFER_BIT);
                glDisable(GL_CULL_FACE);
                glUniform2f(self.u_fog, 1.0e6, 2.0e6);
                let hb = self.hand_b;
                glUniform3f(self.u_tint, hb, hb, hb);
                let proj = math::perspective(1.2, aspect, 0.05, 20.0);
                let sw = self.swing.map_or(0.0, |t| (t / SWING_T).min(1.0));
                if self.shown.map_or(false, |i| i as i32 == self.map_idx) {
                    if let Some(st) = &self.net_state {
                        let mut g = st.lock().unwrap();
                        if g.map_dirty && g.map.len() == 16384 {
                            g.map_dirty = false;
                            let mut px = Vec::with_capacity(16384 * 4);
                            for &b in &g.map {
                                let base = (b >> 2) as usize;
                                if base == 0 || base > 61 {
                                    px.extend_from_slice(&[214, 186, 140, 255]);
                                } else {
                                    let m = [180u32, 220, 255, 135][(b & 3) as usize];
                                    let c = MAP_COLORS[base];
                                    px.extend_from_slice(&[(c[0] as u32 * m / 255) as u8, (c[1] as u32 * m / 255) as u8, (c[2] as u32 * m / 255) as u8, 255]);
                                }
                            }
                            glBindTexture(GL_TEXTURE_2D, self.map_tex);
                            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 128, 128, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const _);
                            glBindTexture(GL_TEXTURE_2D, self.tex);
                        }
                    }
                    let pd = -self.pitch.to_degrees();
                    let f = (1.0 - pd / 45.0).clamp(0.0, 1.0);
                    let tilt = -(f * std::f32::consts::PI).cos() * 0.5 + 0.5;
                    let sq = sw.sqrt();
                    let m = math::mul(
                        &proj,
                        &math::chain(&[
                            math::translate(0.0, 0.04 - self.lower * 1.2 - 0.2 * (sq * std::f32::consts::PI).sin() * 0.0, -0.72),
                            math::rot_x((tilt * -85.0f32).to_radians()),
                            math::rot_x((sw * std::f32::consts::PI).sin() * 20.0f32.to_radians()),
                            math::scale(0.76, 0.76, 0.76),
                        ]),
                    );
                    glUniformMatrix4fv(self.u_mvp, 1, 0, m.as_ptr());
                    glBindTexture(GL_TEXTURE_2D, self.map_tex);
                    bind(&self.map_gpu);
                    glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_INT, 0 as *const _);
                    glBindTexture(GL_TEXTURE_2D, self.tex);
                } else {
                match self.shown.and_then(|i| self.items.get(i as usize)) {
                    Some(it) => {
                        let m = math::mul(&proj, &if it.flat { items::hand_flat_matrix(self.lower, sw) } else { items::hand_matrix(self.lower, sw) });
                        glUniformMatrix4fv(self.u_mvp, 1, 0, m.as_ptr());
                        bind(&self.items_gpu);
                        glDrawElements(GL_TRIANGLES, it.hand.1, GL_UNSIGNED_INT, (it.hand.0 as usize * 4) as *const _);
                    }
                    None => {
                        let m = math::mul(&proj, &items::arm_matrix(self.lower, sw));
                        glUniformMatrix4fv(self.u_mvp, 1, 0, m.as_ptr());
                        glBindTexture(GL_TEXTURE_2D, self.skin_tex);
                        bind(&self.arm_gpu);
                        glDrawElements(GL_TRIANGLES, self.arm_gpu.count, GL_UNSIGNED_INT, 0 as *const _);
                        glBindTexture(GL_TEXTURE_2D, self.tex);
                    }
                }
                }
                glUniform3f(self.u_tint, 1.0, 1.0, 1.0);
            }
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_CULL_FACE);
        }

        // HUD
        self.frames += 1;
        if self.fps_t.elapsed().as_secs_f32() >= 1.0 {
            self.fps = self.frames;
            self.frames = 0;
            self.fps_t = Instant::now();
        }
        let (w, h) = (g.w as f32, g.h as f32);
        let k = gs(g);
        let slots: Vec<(usize, [f32; 4])> = if self.win.is_some() { Vec::new() } else if self.inv_open { (0..SLOTS).map(|i| (i, inv_item_rect(g, i))).collect() } else { (0..9).map(|i| (i, hot_item_rect(g, i))).collect() };

        // lượt 1 (2D): nền tối, chữ HUD, dấu ngắm, nút
        g.begin();
        if self.hurt_t > 0.0 {
            g.rect(0.0, 0.0, w, h, 0xc0000000 | ((self.hurt_t / 0.4 * 110.0) as u32).min(255));
        }
        if self.inv_open || self.win.is_some() {
            g.rect(0.0, 0.0, w, h, 0x101010c8);
            if self.chest_tex.is_none() {
                if let Some(l) = self.win_layout(g) {
                    g.rect(l.panel[0], l.panel[1], l.panel[2], l.panel[3], 0xc6c6c6ff);
                    for r in &l.slots {
                        g.rect(r[0] - l.k, r[1] - l.k, 18.0 * l.k, 18.0 * l.k, 0x8b8b8bff);
                    }
                }
            }
        } else {
            g.text(12.0 * s, 84.0 * s, 13.0 * s, 0xffffffff, &format!("FPS {}  x{:.0} y{:.0} z{:.0}  chunk {}/{}  đỉnh {}k  {}", self.fps, self.pos[0], self.pos[1], self.pos[2], self.drawn_chunks, self.gpu.len(), self.drawn_verts / 1000, if self.player.fly { "bay" } else if self.player.on_ground { "đất" } else { "không" }));
            // dấu ngắm 15x15 đơn vị GUI giữa màn hình
            let (cx, cy) = (w / 2.0, h / 2.0);
            g.rect(cx - 7.5 * k, cy - 0.5 * k, 15.0 * k, k, 0xf0f0f0d0);
            g.rect(cx - 0.5 * k, cy - 7.5 * k, k, 15.0 * k, 0xf0f0f0d0);
            if self.hot_tex.is_none() {
                let (bx, by) = hot_origin(g);
                g.rect(bx, by, 182.0 * k, 22.0 * k, 0x000000b0);
            }
        }
        let on = 0x2e8b57c0u32;
        let off = 0x00000090u32;
        let fly_label = if self.player.fly { "Bay: bật" } else { "Bay: tắt" };
        let modal = self.inv_open || self.win.is_some();
        let bag_label = if modal { "Đóng" } else { "Túi đồ" };
        let mut buttons: Vec<([f32; 4], &str, u32)> = vec![(rect_exit(g), "II", 0x404040d0u32), (rect_bag(g), bag_label, if modal { on } else { off })];
        if !modal {
            buttons.push((rect_jump(g), if self.player.fly { "Lên" } else { "Nhảy" }, if self.jump_held { on } else { off }));
            buttons.push((rect_sneak(g), if self.player.fly { "Xuống" } else { "Lén" }, if self.sneak_on { on } else { off }));
            buttons.push((rect_fly(g), fly_label, if self.player.fly { on } else { off }));
            buttons.push((rect_sprint(g), "Chạy", if self.sprint_on { on } else { off }));
            if self.net.is_some() {
                buttons.push((rect_attack(g), "Đánh", if self.attack_held { 0xb03030c0 } else { off }));
                buttons.push((rect_place(g), "Đặt", off));
            }
        }
        for (r, label, col) in buttons {
            g.rect(r[0], r[1], r[2], r[3], col);
            let tw = g.text_width(label, 14.0 * s);
            g.text(r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - 18.0 * s) / 2.0, 14.0 * s, 0xffffffff, label);
        }
        g.end(None);

        // dò lỗi cửa sổ vật chứa: ghi một dòng mỗi khi cửa sổ đổi
        if let Some(wn) = &self.win {
            static LAST_WIN: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
            let filled = wn.slots.iter().filter(|x| x.is_some()).count();
            let key = ((wn.id as i64) << 32) | ((wn.slots.len() as i64) << 12) | filled as i64;
            if LAST_WIN.swap(key, std::sync::atomic::Ordering::Relaxed) != key {
                let missing: Vec<u32> = wn.slots.iter().flatten().filter(|(id, _, _)| self.items.get(*id as usize).is_none()).map(|(id, _, _)| *id).take(6).collect();
                let lay = self.win_layout(g).map(|l| format!("{:.0},{:.0} {:.0}x{:.0} k={}", l.panel[0], l.panel[1], l.panel[2], l.panel[3], l.k));
                crate::netplay::trace(format!("WINDRAW id={} loại={} ô={} có vật={} vật không có icon={:?} chest_tex={:?} khung={:?} màn={}x{}", wn.id, wn.kind, wn.slots.len(), filled, missing, self.chest_tex.map(|t| (t.1, t.2)), lay, g.w, g.h));
                crate::netplay::trace_flush();
            }
        }

        // lượt 2 (3D/ảnh): nền túi đồ & thanh nhanh bằng ảnh thật, Steve trong khung nhân vật, icon khối
        unsafe {
            glDisable(GL_SCISSOR_TEST);
            glDisable(GL_CULL_FACE);
            glDisable(GL_DEPTH_TEST);
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glUseProgram(self.prog);
            glUniform2f(self.u_fog, 1.0e6, 2.0e6);
            glUniform3f(self.u_tint, 1.0, 1.0, 1.0);
            glActiveTexture(GL_TEXTURE0);
            glUniform1i(self.u_tex, 0);
            let om = items::ortho(w, h);
            glUniformMatrix4fv(self.u_mvp, 1, 0, om.as_ptr());
            if self.win.is_some() {
                if let (Some((t, tw, th)), Some(l)) = (self.chest_tex, self.win_layout(g)) {
                    self.quad(t, l.top_dst, l.top_src, tw, th);
                    self.quad(t, l.bot_dst, l.bot_src, tw, th);
                }
            } else if self.inv_open {
                if let Some((t, tw, th)) = self.inv_tex {
                    let p = inv_panel(g);
                    self.quad(t, [p[0], p[1], p[2], p[3]], [0.0, 0.0, 176.0, 166.0], tw, th);
                }
            } else if !self.chat_open {
                if let Some((t, tw, th, src)) = self.hot_tex {
                    let (bx, by) = hot_origin(g);
                    self.quad(t, [bx, by, 182.0 * k, 22.0 * k], src, tw, th);
                }
                if let Some((t, tw, th, src)) = self.sel_tex {
                    let r = hot_slot_rect(g, self.inv.sel);
                    self.quad(t, [r[0] - k, r[1] - k, 24.0 * k, 24.0 * k], src, tw, th);
                }
                // máu, đói, thanh kinh nghiệm (chế độ sinh tồn: ẩn khi đang bay)
                if !self.player.fly {
                    if let Some(hud) = &self.hud {
                        let cx = w / 2.0;
                        let hy = g.h as f32 - 39.0 * k;
                        let sp = |sv: Spr, x: f32, y: f32, ww: f32, hh: f32| {
                            if let Some(&(t, tw, th)) = self.hud_tex.get(sv.tex) {
                                self.quad(t, [x, y, ww * k, hh * k], sv.r, tw, th);
                            }
                        };
                        let hp = self.health.round() as i32;
                        for i in 0..10 {
                            let x = cx - 91.0 * k + 8.0 * k * i as f32;
                            sp(hud.heart_bg, x, hy, 9.0, 9.0);
                            if hp >= 2 * i + 2 {
                                sp(hud.heart_full, x, hy, 9.0, 9.0);
                            } else if hp == 2 * i + 1 {
                                sp(hud.heart_half, x, hy, 9.0, 9.0);
                            }
                            let fx = cx + 91.0 * k - 8.0 * k * (i + 1) as f32 - k;
                            sp(hud.food_bg, fx, hy, 9.0, 9.0);
                            sp(hud.food_full, fx, hy, 9.0, 9.0);
                        }
                        sp(hud.xp_bg, cx - 91.0 * k, g.h as f32 - 29.0 * k, 182.0, 5.0);
                    }
                }
            }
            glDisable(GL_BLEND);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glDepthMask(1);
            glClear(GL_DEPTH_BUFFER_BIT);
            let stride = std::mem::size_of::<V>() as i32;
            let attrs = |vbo: u32, ebo: u32| {
                glBindBuffer(GL_ARRAY_BUFFER, vbo);
                glEnableVertexAttribArray(0);
                glVertexAttribPointer(0, 3, GL_FLOAT, 0, stride, 0 as *const _);
                glEnableVertexAttribArray(1);
                glVertexAttribPointer(1, 2, GL_FLOAT, 0, stride, 12 as *const _);
                glEnableVertexAttribArray(2);
                glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, 1, stride, 20 as *const _);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ebo);
            };
            // Steve trong khung nhân vật (vị trí 26..75 x 8..75, chân ở (51, 75)), thân và đầu quay theo ngón tay như vanilla
            if self.inv_open && self.inv_tex.is_some() {
                let p = inv_panel(g);
                let ks = 30.0 / 16.0 * 0.9375 * k;
                let fx = ((p[0] + 51.0 * k - self.ui_pt.0) / k / 40.0).atan();
                let fy = ((p[1] + 25.0 * k - self.ui_pt.1) / k / 40.0).atan();
                let (mb, mh) = items::body_matrices(w, h, p[0] + 51.0 * k, p[1] + 75.0 * k, ks, fx * 20f32.to_radians(), fx * 20f32.to_radians(), -fy * 20f32.to_radians());
                glBindTexture(GL_TEXTURE_2D, self.skin_tex);
                attrs(self.body_gpu.vbo, self.body_gpu.ebo);
                let hn = self.head_n;
                glUniformMatrix4fv(self.u_mvp, 1, 0, mh.as_ptr());
                glDrawElements(GL_TRIANGLES, hn, GL_UNSIGNED_INT, 0 as *const _);
                glUniformMatrix4fv(self.u_mvp, 1, 0, mb.as_ptr());
                glDrawElements(GL_TRIANGLES, self.body_gpu.count - hn, GL_UNSIGNED_INT, (hn as usize * 4) as *const _);
            }
            if !self.items.is_empty() {
                glBindTexture(GL_TEXTURE_2D, self.tex);
                attrs(self.items_gpu.vbo, self.items_gpu.ebo);
                let draw = |item: u16, cx: f32, cy: f32, sz: f32, zoff: f32| {
                    if let Some(it) = self.items.get(item as usize) {
                        let m = if it.flat { items::flat_matrix(w, h, cx, cy, sz, zoff) } else { items::gui_matrix(w, h, cx, cy, sz, zoff) };
                        glUniformMatrix4fv(self.u_mvp, 1, 0, m.as_ptr());
                        glDrawElements(GL_TRIANGLES, it.gui.1, GL_UNSIGNED_INT, (it.gui.0 as usize * 4) as *const _);
                    }
                };
                for (i, r) in &slots {
                    if let Some(st) = self.inv.slots[*i] {
                        draw(st.item, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, 16.0 * k, 0.0);
                    }
                }
                if self.inv_open {
                    if let Some(c) = self.inv.cursor {
                        // vật đang cầm nằm phía trên ngón tay để không bị che
                        draw(c.item, self.ui_pt.0, self.ui_pt.1 - 14.0 * k, 16.0 * k, 16.0 * k * 3.0);
                    }
                }
                if let (Some(wn), Some(l)) = (&self.win, self.win_layout(g)) {
                    for (i, r) in l.slots.iter().enumerate() {
                        if let Some(Some((id, _, _))) = wn.slots.get(i) {
                            draw(*id as u16, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, 16.0 * k, 0.0);
                        }
                    }
                }
            }
            glDisable(GL_DEPTH_TEST);
        }

        // lượt 3 (2D): đánh dấu ô, khung chọn dự phòng, số lượng, tên vật
        g.begin();
        if self.inv_open {
            // ô đang bị ngón tay chạm: phủ trắng mờ như vanilla
            if self.touches.iter().any(|t| t.role == Role::Ui) {
                if let Some(i) = (0..SLOTS).find(|&i| inside(inv_slot_rect(g, i), self.ui_pt.0, self.ui_pt.1)) {
                    let r = inv_item_rect(g, i);
                    g.rect(r[0], r[1], r[2], r[3], 0x80ffffff);
                }
            }
        } else if self.win.is_none() {
            if self.sel_tex.is_none() {
                let r = hot_slot_rect(g, self.inv.sel);
                let t = 2.0 * k;
                for q in [[r[0], r[1], r[2], t], [r[0], r[1] + r[3] - t, r[2], t], [r[0], r[1], t, r[3]], [r[0] + r[2] - t, r[1], t, r[3]]] {
                    g.rect(q[0], q[1], q[2], q[3], 0xffffffff);
                }
            }
            if self.name_t > 0.0 {
                if let Some(it) = self.shown.or(self.inv.held()).and_then(|i| self.items.get(i as usize)) {
                    let a = (255.0 * (self.name_t / 0.5).min(1.0)) as u32;
                    let size = 9.0 * k;
                    let tw = g.text_width(&it.name, size);
                    let (tx, ty) = ((w - tw) / 2.0, hot_origin(g).1 - 24.0 * k);
                    g.text(tx + k, ty + k, size, 0x3f3f3f00 | a, &it.name);
                    g.text(tx, ty, size, 0xffffff00 | a, &it.name);
                }
            }
        }
        for (i, r) in &slots {
            if let Some(st) = self.inv.slots[*i] {
                draw_count(g, *r, st.n);
            }
        }
        if self.inv_open {
            if let Some(c) = self.inv.cursor {
                let sz = 16.0 * k;
                draw_count(g, [self.ui_pt.0 - sz / 2.0, self.ui_pt.1 - 14.0 * k - sz / 2.0, sz, sz], c.n);
            }
        }
        if let (Some(wn), Some(l)) = (&self.win, self.win_layout(g)) {
            let ts = 9.0 * l.k;
            let title = crate::winui::strip_codes(&wn.title);
            if !title.is_empty() {
                g.text(l.title_at.0, l.title_at.1, ts, 0x404040ff, &title);
            }
            g.text(l.label_at.0, l.label_at.1, ts, 0x404040ff, "Túi đồ");
            for (i, r) in l.slots.iter().enumerate() {
                if let Some(Some((_, n, _))) = wn.slots.get(i) {
                    draw_count(g, *r, *n);
                }
            }
            // ngón tay đang chạm: tô sáng ô và hiện tên vật (nhả ngón để bấm)
            if let Some((px, py)) = self.win_pt {
                if let Some(i) = crate::winui::hit(&l, px, py) {
                    let r = l.slots[i];
                    g.rect(r[0], r[1], r[2], r[3], 0x80ffffff);
                    if let Some(Some((id, _, nm))) = wn.slots.get(i) {
                        let name = nm.clone().or_else(|| self.items.get(*id as usize).map(|it| it.name.clone())).unwrap_or_default();
                        if !name.is_empty() {
                            let size = 9.0 * l.k;
                            let tw = g.text_width(&crate::winui::strip_codes(&name), size);
                            let (bw, bh) = (tw + 8.0 * l.k, size * 1.25 + 6.0 * l.k);
                            let tx = (px + 12.0 * l.k).min(w - bw).max(0.0);
                            let ty = (py - 12.0 * l.k - bh).max(0.0);
                            g.rect(tx - l.k, ty - l.k, bw + 2.0 * l.k, bh + 2.0 * l.k, 0x2a0066ff);
                            g.rect(tx, ty, bw, bh, 0x100010f0);
                            draw_legacy(g, tx + 4.0 * l.k, ty + 3.0 * l.k, size, &name, 255);
                        }
                    }
                }
            }
        }
        g.end(None);
        self.chat_pass(g);
        std::mem::take(&mut self.exit)
    }
}
