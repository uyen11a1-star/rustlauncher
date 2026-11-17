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
    math,
    phys::{Collider, Input, Player, TICK},
    scene::{pack, ChunkMesh, Msg, SceneData, Streamer},
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
out vec4 o;
void main(){
  vec4 t=texture(u_tex,v_uv); if(t.a<0.1) discard;
  float f=clamp((u_fog.y-v_d)/(u_fog.y-u_fog.x),0.0,1.0);
  vec3 c=mix(u_fogc, t.rgb*v_col.rgb, f);
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
}

struct Touch {
    id: i32,
    role: Role,
    sx: f32,
    sy: f32,
    x: f32,
    y: f32,
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
            Ok(View {
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
                let role = if inside(rect_exit(g), x, y) {
                    Role::Exit
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
                self.touches.push(Touch { id, role, sx: x, sy: y, x, y });
            }
            Phase::Move => {
                let sens = 0.0035; // rad / pixel
                if let Some(t) = self.touches.iter_mut().find(|t| t.id == id) {
                    if t.role == Role::Look {
                        self.yaw -= (x - t.x) * sens;
                        self.pitch = (self.pitch - (y - t.y) * sens).clamp(-1.5, 1.5);
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
        g.begin();
        g.text(12.0 * s, 84.0 * s, 13.0 * s, 0xffffffff, &format!("FPS {}  x{:.0} y{:.0} z{:.0}  chunk {}/{}  đỉnh {}k  {}", self.fps, self.pos[0], self.pos[1], self.pos[2], self.drawn_chunks, self.gpu.len(), self.drawn_verts / 1000, if self.player.fly { "bay" } else if self.player.on_ground { "đất" } else { "không" }));
        let on = 0x2e8b57c0u32;
        let off = 0x00000090u32;
        let fly_label = if self.player.fly { "Bay: bật" } else { "Bay: tắt" };
        for (r, label, col) in [
            (rect_exit(g), "Thoát", 0xc0392bd0u32),
            (rect_jump(g), if self.player.fly { "Lên" } else { "Nhảy" }, if self.jump_held { on } else { off }),
            (rect_sneak(g), if self.player.fly { "Xuống" } else { "Lén" }, if self.sneak_on { on } else { off }),
            (rect_fly(g), fly_label, if self.player.fly { on } else { off }),
            (rect_sprint(g), "Chạy", if self.sprint_on { on } else { off }),
        ] {
            g.rect(r[0], r[1], r[2], r[3], col);
            let tw = g.text_width(label, 14.0 * s);
            g.text(r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - 18.0 * s) / 2.0, 14.0 * s, 0xffffffff, label);
        }
        g.end(None);
        self.exit
    }
}
