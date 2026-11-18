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
    inv::Inv,
    items::{self, Item, SWING_T},
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

/// Số lượng ở góc dưới-phải ô (có bóng đen), ẩn khi chỉ có 1.
fn draw_count(g: &mut Gfx, r: [f32; 4], n: u8) {
    if n <= 1 {
        return;
    }
    let s = g.scale;
    let size = r[2] * 0.34;
    let txt = n.to_string();
    let tw = g.text_width(&txt, size);
    let (x, y) = (r[0] + r[2] - tw - 3.0 * s, r[1] + r[3] - size * 1.3 - 1.0 * s);
    g.text(x + 0.8 * s, y + 0.8 * s, size, 0x000000ff, &txt);
    g.text(x, y, size, 0xffffffff, &txt);
}

fn rect_bag(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 92.0 * s, 84.0 * s, 80.0 * s, 40.0 * s]
}

/// Thanh nhanh: (x trái, y trên, cỡ ô).
fn hot_geom(g: &Gfx) -> (f32, f32, f32) {
    let s = g.scale;
    let slot = ((g.w as f32 - 16.0 * s) / 9.0).min(44.0 * s);
    ((g.w as f32 - 9.0 * slot) / 2.0, g.h as f32 - slot - 8.0 * s, slot)
}

/// Khung túi đồ: (x, y, rộng, cao, cỡ ô, lề, cao tiêu đề, khoảng cách giữa túi và thanh nhanh).
fn inv_geom(g: &Gfx) -> (f32, f32, f32, f32, f32, f32, f32, f32) {
    let s = g.scale;
    let slot = ((g.h as f32 - 100.0 * s) / 4.5).min((g.w as f32 - 24.0 * s) / 9.4).min(46.0 * s).max(20.0 * s);
    let (pad, title, gap) = (10.0 * s, 26.0 * s, 10.0 * s);
    let (pw, ph) = (9.0 * slot + 2.0 * pad, title + 4.0 * slot + gap + 2.0 * pad);
    ((g.w as f32 - pw) / 2.0, (g.h as f32 - ph) / 2.0, pw, ph, slot, pad, title, gap)
}

/// Hình chữ nhật ô túi `i` (0..8 thanh nhanh, 9..35 túi) khi mở túi đồ.
fn inv_slot_rect(g: &Gfx, i: usize) -> [f32; 4] {
    let (px, py, _, _, slot, pad, title, gap) = inv_geom(g);
    if i < 9 {
        [px + pad + i as f32 * slot, py + pad + title + 3.0 * slot + gap, slot, slot]
    } else {
        let (r, c) = ((i - 9) / 9, (i - 9) % 9);
        [px + pad + c as f32 * slot, py + pad + title + r as f32 * slot, slot, slot]
    }
}

fn hot_slot_rect(g: &Gfx, i: usize) -> [f32; 4] {
    let (x, y, slot) = hot_geom(g);
    [x + i as f32 * slot, y, slot, slot]
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
            let items_gpu = upload_raw(&d.items.v, &d.items.i);
            let arm_gpu = upload_raw(&d.items.arm_v, &d.items.arm_i);
            let skin_tex = make_tex(&d.items.skin.rgba, d.items.skin.w, d.items.skin.h);
            glBindTexture(GL_TEXTURE_2D, tex);
            let n_items = d.items.list.len();
            Ok(View {
                u_tint,
                items: d.items.list,
                items_gpu,
                arm_gpu,
                skin_tex,
                inv: Inv::new(n_items),
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
                    self.ui_down = (0..36).find(|&i| inside(inv_slot_rect(g, i), x, y)).map(|i| {
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
                            } else {
                                self.inv.stash();
                            }
                        }
                        Role::Hot(i) if inside(hot_slot_rect(g, i), x, y) => self.inv.sel = i,
                        Role::Look if t.moved < 14.0 * g.scale && t.t0.elapsed().as_secs_f32() < 0.3 && self.swing.is_none() => self.swing = Some(0.0),
                        Role::Ui => {
                            let up = (0..36).find(|&i| inside(inv_slot_rect(g, i), x, y));
                            match (self.ui_down.take(), up) {
                                // đang cầm vật: thả vào ô dưới ngón tay
                                (Some((_, false)), Some(u)) => self.inv.click(u),
                                // kéo từ ô này sang ô khác: thả
                                (Some((d, true)), Some(u)) if d != u => self.inv.click(u),
                                // thả ra ngoài khung túi khi đang cầm: vứt bỏ
                                (_, None) if self.inv.cursor.is_some() => {
                                    let (px, py, pw, ph, ..) = inv_geom(g);
                                    if !inside([px, py, pw, ph], x, y) {
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
        let slots: Vec<(usize, [f32; 4])> = if self.inv_open { (0..36).map(|i| (i, inv_slot_rect(g, i))).collect() } else { (0..9).map(|i| (i, hot_slot_rect(g, i))).collect() };

        // lượt 1 (2D): nền túi đồ / thanh nhanh, nút, chữ HUD
        g.begin();
        if self.inv_open {
            g.rect(0.0, 0.0, w, h, 0x000000a0);
            let (px, py, pw, ph, _slot, pad, _title, _gap) = inv_geom(g);
            g.rect(px - 2.0 * s, py - 2.0 * s, pw + 4.0 * s, ph + 4.0 * s, 0x000000ff);
            g.rect(px, py, pw, ph, 0xc6c6c6ff);
            g.text(px + pad, py + pad * 0.7, 15.0 * s, 0x404040ff, "Túi đồ");
            for (_, r) in &slots {
                g.rect(r[0], r[1], r[2], r[3], 0x373737ff);
                g.rect(r[0] + 2.0 * s, r[1] + 2.0 * s, r[2] - 4.0 * s, r[3] - 4.0 * s, 0x8b8b8bff);
            }
        } else {
            g.text(12.0 * s, 84.0 * s, 13.0 * s, 0xffffffff, &format!("FPS {}  x{:.0} y{:.0} z{:.0}  chunk {}/{}  đỉnh {}k  {}", self.fps, self.pos[0], self.pos[1], self.pos[2], self.drawn_chunks, self.gpu.len(), self.drawn_verts / 1000, if self.player.fly { "bay" } else if self.player.on_ground { "đất" } else { "không" }));
            for (_, r) in &slots {
                g.rect(r[0], r[1], r[2], r[3], 0x000000a0);
                g.rect(r[0] + 1.0 * s, r[1] + 1.0 * s, r[2] - 2.0 * s, r[3] - 2.0 * s, 0x55555580);
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

        // lượt 2 (3D): icon khối trong các ô (và vật đang cầm theo ngón tay)
        if !self.items.is_empty() {
            unsafe {
                glDisable(GL_SCISSOR_TEST);
                glDisable(GL_BLEND);
                glDisable(GL_CULL_FACE);
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LEQUAL);
                glDepthMask(1);
                glClear(GL_DEPTH_BUFFER_BIT);
                glUseProgram(self.prog);
                glUniform2f(self.u_fog, 1.0e6, 2.0e6);
                glUniform3f(self.u_tint, 1.0, 1.0, 1.0);
                glActiveTexture(GL_TEXTURE0);
                glBindTexture(GL_TEXTURE_2D, self.tex);
                glUniform1i(self.u_tex, 0);
                let stride = std::mem::size_of::<V>() as i32;
                glBindBuffer(GL_ARRAY_BUFFER, self.items_gpu.vbo);
                glEnableVertexAttribArray(0);
                glVertexAttribPointer(0, 3, GL_FLOAT, 0, stride, 0 as *const _);
                glEnableVertexAttribArray(1);
                glVertexAttribPointer(1, 2, GL_FLOAT, 0, stride, 12 as *const _);
                glEnableVertexAttribArray(2);
                glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, 1, stride, 20 as *const _);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, self.items_gpu.ebo);
                let draw = |item: u16, cx: f32, cy: f32, sz: f32, zoff: f32| {
                    if let Some(it) = self.items.get(item as usize) {
                        let m = items::gui_matrix(w, h, cx, cy, sz, zoff);
                        glUniformMatrix4fv(self.u_mvp, 1, 0, m.as_ptr());
                        glDrawElements(GL_TRIANGLES, it.gui.1, GL_UNSIGNED_INT, (it.gui.0 as usize * 4) as *const _);
                    }
                };
                for (i, r) in &slots {
                    if let Some(st) = self.inv.slots[*i] {
                        draw(st.item, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2] * 0.8, 0.0);
                    }
                }
                if self.inv_open {
                    if let Some(c) = self.inv.cursor {
                        let sz = inv_geom(g).4 * 0.8;
                        draw(c.item, self.ui_pt.0, self.ui_pt.1, sz, sz * 3.0);
                    }
                }
                glDisable(GL_DEPTH_TEST);
            }
        }

        // lượt 3 (2D): khung chọn, số lượng, tên vật
        g.begin();
        if !self.inv_open {
            let r = hot_slot_rect(g, self.inv.sel);
            let t = 3.0 * s;
            for q in [[r[0] - t, r[1] - t, r[2] + 2.0 * t, t], [r[0] - t, r[1] + r[3], r[2] + 2.0 * t, t], [r[0] - t, r[1], t, r[3]], [r[0] + r[2], r[1], t, r[3]]] {
                g.rect(q[0], q[1], q[2], q[3], 0xffffffff);
            }
            if self.name_t > 0.0 {
                if let Some(it) = self.shown.or(self.inv.held()).and_then(|i| self.items.get(i as usize)) {
                    let a = (255.0 * (self.name_t / 0.5).min(1.0)) as u32;
                    let tw = g.text_width(&it.name, 15.0 * s);
                    let (tx, ty) = ((w - tw) / 2.0, hot_geom(g).1 - 30.0 * s);
                    g.text(tx + s, ty + s, 15.0 * s, a, &it.name);
                    g.text(tx, ty, 15.0 * s, 0xffffff00 | a, &it.name);
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
                let sz = inv_geom(g).4;
                draw_count(g, [self.ui_pt.0 - sz / 2.0, self.ui_pt.1 - sz / 2.0, sz, sz], c.n);
            }
        }
        g.end(None);
        self.exit
    }
}
