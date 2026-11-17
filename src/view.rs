//! Màn hình xem cảnh 3D (G1): renderer GLES3 tự viết + điều khiển cảm ứng + HUD.
use std::{ffi::CString, time::Instant};

use crate::{
    assets::V,
    gfx::{build_program, Gfx},
    gles::*,
    math,
    scene::SceneData,
};

const VS: &str = "#version 300 es
in vec3 a_pos; in vec2 a_uv; in vec4 a_col;
uniform mat4 u_mvp;
out vec2 v_uv; out vec4 v_col;
void main(){ v_uv=a_uv; v_col=a_col; gl_Position=u_mvp*vec4(a_pos,1.0); }";

const FS: &str = "#version 300 es
precision highp float;
in vec2 v_uv; in vec4 v_col;
uniform sampler2D u_tex;
out vec4 o;
void main(){ vec4 t=texture(u_tex,v_uv); if(t.a<0.1) discard; o=vec4(t.rgb*v_col.rgb,1.0); }";

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Move,
    Look,
    Up,
    Down,
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
    vbo: u32,
    ebo: u32,
    tex: u32,
    u_mvp: i32,
    u_tex: i32,
    count: i32,
    nverts: usize,
    pos: [f32; 3],
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
fn rect_up(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 76.0 * s, g.h as f32 - 190.0 * s, 64.0 * s, 64.0 * s]
}
fn rect_down(g: &Gfx) -> [f32; 4] {
    let s = g.scale;
    [g.w as f32 - 76.0 * s, g.h as f32 - 110.0 * s, 64.0 * s, 64.0 * s]
}
fn inside(r: [f32; 4], x: f32, y: f32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

impl View {
    /// Cần context GL đang hiện hành.
    pub fn new(d: SceneData) -> Result<View, String> {
        let prog = build_program(VS, FS, &["a_pos", "a_uv", "a_col"])?;
        unsafe {
            let c = |s: &str| CString::new(s).unwrap();
            let u_mvp = glGetUniformLocation(prog, c("u_mvp").as_ptr());
            let u_tex = glGetUniformLocation(prog, c("u_tex").as_ptr());
            let mut vbo = 0;
            glGenBuffers(1, &mut vbo);
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBufferData(GL_ARRAY_BUFFER, (d.verts.len() * std::mem::size_of::<V>()) as isize, d.verts.as_ptr() as *const _, GL_STATIC_DRAW);
            let mut ebo = 0;
            glGenBuffers(1, &mut ebo);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ebo);
            glBufferData(GL_ELEMENT_ARRAY_BUFFER, (d.idx.len() * 4) as isize, d.idx.as_ptr() as *const _, GL_STATIC_DRAW);
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
                vbo,
                ebo,
                tex,
                u_mvp,
                u_tex,
                count: d.idx.len() as i32,
                nverts: d.verts.len(),
                pos: d.spawn,
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
                } else if inside(rect_up(g), x, y) {
                    Role::Up
                } else if inside(rect_down(g), x, y) {
                    Role::Down
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
                    if t.role == Role::Exit && inside(rect_exit(g), x, y) {
                        self.exit = true;
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

        // di chuyển: cần trái = (dịch chuyển ngón tay so với lúc chạm) / 70dp
        let (mut sx, mut sy) = (0.0f32, 0.0f32);
        let mut vy = 0.0f32;
        for t in &self.touches {
            match t.role {
                Role::Move => {
                    sx = ((t.x - t.sx) / (70.0 * s)).clamp(-1.0, 1.0);
                    sy = ((t.y - t.sy) / (70.0 * s)).clamp(-1.0, 1.0);
                }
                Role::Up => vy += 1.0,
                Role::Down => vy -= 1.0,
                _ => {}
            }
        }
        let speed = 14.0;
        let (sn, cs) = self.yaw.sin_cos();
        let fwd = [-sn, -cs];
        let right = [cs, -sn];
        self.pos[0] += (fwd[0] * -sy + right[0] * sx) * speed * dt;
        self.pos[2] += (fwd[1] * -sy + right[1] * sx) * speed * dt;
        self.pos[1] += vy * speed * dt;

        let aspect = g.w as f32 / g.h as f32;
        let mvp = math::mul(&math::perspective(1.2, aspect, 0.05, 300.0), &math::view(self.pos, self.yaw, self.pitch));
        unsafe {
            glViewport(0, 0, g.w, g.h);
            glDisable(GL_SCISSOR_TEST);
            glClearColor(0.53, 0.81, 0.92, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glDisable(GL_BLEND);
            glEnable(GL_CULL_FACE);
            glUseProgram(self.prog);
            glUniformMatrix4fv(self.u_mvp, 1, 0, mvp.as_ptr());
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, self.tex);
            glUniform1i(self.u_tex, 0);
            glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
            let stride = std::mem::size_of::<V>() as i32;
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, stride, 0 as *const _);
            glEnableVertexAttribArray(1);
            glVertexAttribPointer(1, 2, GL_FLOAT, 0, stride, 12 as *const _);
            glEnableVertexAttribArray(2);
            glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, 1, stride, 20 as *const _);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, self.ebo);
            glDrawElements(GL_TRIANGLES, self.count, GL_UNSIGNED_INT, 0 as *const _);
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
        g.text(12.0 * s, 36.0 * s, 13.0 * s, 0xffffffff, &format!("FPS {}  x {:.1} y {:.1} z {:.1}  ({} đỉnh)", self.fps, self.pos[0], self.pos[1], self.pos[2], self.nverts));
        for (r, label, col) in [
            (rect_exit(g), "Thoát", 0xc0392bd0u32),
            (rect_up(g), "Lên", 0x00000090),
            (rect_down(g), "Xuống", 0x00000090),
        ] {
            g.rect(r[0], r[1], r[2], r[3], col);
            let tw = g.text_width(label, 14.0 * s);
            g.text(r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - 18.0 * s) / 2.0, 14.0 * s, 0xffffffff, label);
        }
        g.end(None);
        self.exit
    }
}
