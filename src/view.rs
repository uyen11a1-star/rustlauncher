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
            let hot_tex = d.items.hot.as_ref().map(|(t, r)| (make_tex(&t.rgba, t.w, t.h), t.w as f32, t.h as f32, *r));
            let sel_tex = d.items.sel.as_ref().map(|(t, r)| (make_tex(&t.rgba, t.w, t.h), t.w as f32, t.h as f32, *r));
            let body_gpu = upload_raw(&d.items.body_v, &d.items.body_i);
            let hud_tex: Vec<(u32, f32, f32)> = d.items.hud_tex.iter().map(|t| (make_tex(&t.rgba, t.w, t.h), t.w as f32, t.h as f32)).collect();
            let mut quad_vbo = 0;
            glGenBuffers(1, &mut quad_vbo);
            let items_gpu = upload_raw(&d.items.v, &d.items.i);
            let arm_gpu = upload_raw(&d.items.arm_v, &d.items.arm_i);
            let skin_tex = make_tex(&d.items.skin.rgba, d.items.skin.w, d.items.skin.h);
            glBindTexture(GL_TEXTURE_2D, tex);
            let n_items = d.items.list.len();
            Ok(View {
                u_tint,
                inv_tex,
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

    pub fn touch(&mut self, id: i32, phase: Phase, x: f32, y: f32, g: &Gfx) {
        match phase {
            Phase::Down => {
                let hot = if self.inv_open { None } else { (0..9).find(|&i| inside(hot_slot_rect(g, i), x, y)) };
                let role = if inside(rect_exit(g), x, y) {
                    Role::Exit
                } else if inside(rect_bag(g), x, y) {
                    Role::Bag
                } else if self.inv_open {
                    Role::Ui
                } else if let Some(i) = hot {
                    Role::Hot(i)
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
                        let pick = self.inv.cursor.is_none() && self.inv.slots[i].is_some();
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
                        Role::Hot(i) if inside(hot_slot_rect(g, i), x, y) => self.inv.sel = i,
                        Role::Look if t.moved < 14.0 * g.scale && t.t0.elapsed().as_secs_f32() < 0.3 && self.swing.is_none() => self.swing = Some(0.0),
                        Role::Ui => {
                            let up = (0..SLOTS).find(|&i| inside(inv_slot_rect(g, i), x, y));
                            match (self.ui_down.take(), up) {
                                // đang cầm vật: thả vào ô dưới ngón tay
                                (Some((_, false)), Some(u)) => self.inv.click(u),
                                // kéo từ ô này sang ô khác: thả
                                (Some((d, true)), Some(u)) if d != u => self.inv.click(u),
                                // thả ra ngoài khung túi khi đang cầm: vứt bỏ
                                (_, None) if self.inv.cursor.is_some() => {
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

    /// Vẽ một khung hình. Trả về true khi người dùng bấm Thoát.
    pub fn frame(&mut self, g: &mut Gfx) -> bool {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        let s = g.scale;

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
            }
        }

        // đầu vào: cần trái = (dịch chuyển ngón tay so với lúc chạm) / 70dp; kéo lên = tiến
        let (mut sx, mut sy) = (0.0f32, 0.0f32);
        self.jump_held = false;
        for t in &self.touches {
            match t.role {
                Role::Move => {
                    sx = ((t.x - t.sx) / (70.0 * s)).clamp(-1.0, 1.0);
                    sy = ((t.y - t.sy) / (70.0 * s)).clamp(-1.0, 1.0);
                }
                Role::Jump => self.jump_held = true,
                _ => {}
            }
        }
        let inp = Input { fwd: (-sy) as f64, strafe: sx as f64, jump: self.jump_held, sneak: self.sneak_on, sprint: self.sprint_on };

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
                // rơi: sát thương = quãng rơi - 3 (làm tròn lên), nước và bay thì không
                if self.col.is_some() && !self.player.fly {
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
                if self.player.pos[1] < -100.0 {
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
        let mvp = math::mul(&math::perspective(1.2, aspect, 0.05, 300.0), &math::view(self.pos, self.yaw, self.pitch));
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

            // tay Steve / khối cầm: vẽ sau cùng, xoá độ sâu để không bị khối thế giới che
            if !self.inv_open {
                glClear(GL_DEPTH_BUFFER_BIT);
                glDisable(GL_CULL_FACE);
                glUniform2f(self.u_fog, 1.0e6, 2.0e6);
                let hb = self.hand_b;
                glUniform3f(self.u_tint, hb, hb, hb);
                let proj = math::perspective(1.2, aspect, 0.05, 20.0);
                let sw = self.swing.map_or(0.0, |t| (t / SWING_T).min(1.0));
                match self.shown.and_then(|i| self.items.get(i as usize)) {
                    Some(it) => {
                        let m = math::mul(&proj, &items::hand_matrix(self.lower, sw));
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
        let slots: Vec<(usize, [f32; 4])> = if self.inv_open { (0..SLOTS).map(|i| (i, inv_item_rect(g, i))).collect() } else { (0..9).map(|i| (i, hot_item_rect(g, i))).collect() };

        // lượt 1 (2D): nền tối, chữ HUD, dấu ngắm, nút
        g.begin();
        if self.hurt_t > 0.0 {
            g.rect(0.0, 0.0, w, h, 0xc0000000 | ((self.hurt_t / 0.4 * 110.0) as u32).min(255));
        }
        if self.inv_open {
            g.rect(0.0, 0.0, w, h, 0x101010c8);
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
        let bag_label = if self.inv_open { "Đóng" } else { "Túi đồ" };
        let mut buttons: Vec<([f32; 4], &str, u32)> = vec![(rect_exit(g), "Thoát", 0xc0392bd0u32), (rect_bag(g), bag_label, if self.inv_open { on } else { off })];
        if !self.inv_open {
            buttons.push((rect_jump(g), if self.player.fly { "Lên" } else { "Nhảy" }, if self.jump_held { on } else { off }));
            buttons.push((rect_sneak(g), if self.player.fly { "Xuống" } else { "Lén" }, if self.sneak_on { on } else { off }));
            buttons.push((rect_fly(g), fly_label, if self.player.fly { on } else { off }));
            buttons.push((rect_sprint(g), "Chạy", if self.sprint_on { on } else { off }));
        }
        for (r, label, col) in buttons {
            g.rect(r[0], r[1], r[2], r[3], col);
            let tw = g.text_width(label, 14.0 * s);
            g.text(r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - 18.0 * s) / 2.0, 14.0 * s, 0xffffffff, label);
        }
        g.end(None);

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
            if self.inv_open {
                if let Some((t, tw, th)) = self.inv_tex {
                    let p = inv_panel(g);
                    self.quad(t, [p[0], p[1], p[2], p[3]], [0.0, 0.0, 176.0, 166.0], tw, th);
                }
            } else {
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
                        let m = items::gui_matrix(w, h, cx, cy, sz, zoff);
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
        } else {
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
        g.end(None);
        self.exit
    }
}
