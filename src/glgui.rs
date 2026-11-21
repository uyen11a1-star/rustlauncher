//! Mặt vẽ GLES cho giao diện kiểu Minecraft: ảnh RGBA lấy mẫu gần nhất, gộp lô theo texture/vùng cắt, panorama bằng shader.
use std::{ffi::CString, ptr::null};

use crate::{gfx::build_program, gles::*, mcgui::canvas::{Canvas, Xf}, png::Image};

#[repr(C)]
#[derive(Clone, Copy)]
struct V {
    x: f32,
    y: f32,
    u: f32,
    v: f32,
    c: [u8; 4],
}

const VS: &str = "#version 300 es
in vec2 a_pos; in vec2 a_uv; in vec4 a_col;
uniform vec2 u_size;
out vec2 v_uv; out vec4 v_col;
void main(){ v_uv=a_uv; v_col=a_col;
  gl_Position=vec4(2.0*a_pos.x/u_size.x-1.0, 1.0-2.0*a_pos.y/u_size.y, 0.0, 1.0); }";
const FS: &str = "#version 300 es
precision mediump float;
in vec2 v_uv; in vec4 v_col;
uniform sampler2D u_tex;
out vec4 o;
void main(){ o = texture(u_tex, v_uv) * v_col; }";

const PVS: &str = "#version 300 es
in vec2 a_pos;
out vec2 v_p;
void main(){ v_p=a_pos; gl_Position=vec4(a_pos,0.0,1.0); }";
const PFS: &str = "#version 300 es
precision highp float;
in vec2 v_p;
uniform sampler2D t0; uniform sampler2D t1; uniform sampler2D t2; uniform sampler2D t3; uniform sampler2D t4; uniform sampler2D t5;
uniform vec3 u_f; uniform vec3 u_r; uniform vec3 u_u; uniform vec2 u_k; uniform float u_a; uniform vec2 u_b; uniform int u_n;
out vec4 o;
vec3 cube(vec3 d){
  vec3 a = abs(d); vec2 uv; vec4 c;
  if (a.y>=a.x && a.y>=a.z) { float s=1.0/a.y;
    if (d.y>0.0) { uv=vec2((d.x*s+1.0)/2.0,(d.z*s+1.0)/2.0); c=textureLod(t4,uv,0.0); }
    else { uv=vec2((d.x*s+1.0)/2.0,(1.0-d.z*s)/2.0); c=textureLod(t5,uv,0.0); }
  } else if (a.z>=a.x) { float s=1.0/a.z;
    if (d.z>0.0) { uv=vec2((d.x*s+1.0)/2.0,(1.0-d.y*s)/2.0); c=textureLod(t0,uv,0.0); }
    else { uv=vec2((1.0-d.x*s)/2.0,(1.0-d.y*s)/2.0); c=textureLod(t2,uv,0.0); }
  } else { float s=1.0/a.x;
    if (d.x>0.0) { uv=vec2((1.0-d.z*s)/2.0,(1.0-d.y*s)/2.0); c=textureLod(t1,uv,0.0); }
    else { uv=vec2((1.0+d.z*s)/2.0,(1.0-d.y*s)/2.0); c=textureLod(t3,uv,0.0); }
  }
  return c.rgb;
}
void main(){
  vec2 off[12] = vec2[12](vec2(0.5,0.0),vec2(0.25,0.433),vec2(-0.25,0.433),vec2(-0.5,0.0),vec2(-0.25,-0.433),vec2(0.25,-0.433),
    vec2(1.0,0.0),vec2(0.5,0.866),vec2(-0.5,0.866),vec2(-1.0,0.0),vec2(-0.5,-0.866),vec2(0.5,-0.866));
  vec3 sum = vec3(0.0); float n = 0.0;
  for (int i = 0; i < 13; i++) {
    if (i >= u_n) break;
    vec2 p = v_p; if (i > 0) p += off[i-1]*u_b;
    vec3 d = u_f + u_r*(p.x*u_k.x) + u_u*(p.y*u_k.y);
    sum += cube(d); n += 1.0;
  }
  o = vec4(sum/n, u_a);
}";

struct Cmd {
    tex: u32,
    clip: Option<[i32; 4]>,
    first: i32,
    count: i32,
}

pub struct GlCanvas {
    prog: u32,
    u_size: i32,
    u_tex: i32,
    pprog: u32,
    vbo: u32,
    texs: Vec<u32>,
    white: u32,
    w: i32,
    h: i32,
    xf: Xf,
    clip: Option<[f32; 4]>,
    verts: Vec<V>,
    cmds: Vec<Cmd>,
    // panorama chờ vẽ ngay (vẽ trước mọi thứ khác trong khung)
    pano: Option<([usize; 6], f32, f32, f32, f32, f32)>,
    /// chẩn đoán panorama (ghi vào nhật ký một lần)
    pub diag: Option<String>,
    frames: u32,
    sizes: Vec<(f32, f32)>,
    /// xoá khung hình trước khi vẽ (tắt khi vẽ đè lên cảnh game)
    pub clear: bool,
}

fn upload(img: &Image, linear: bool) -> u32 {
    unsafe {
        let mut t = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, img.w as i32, img.h as i32, 0, GL_RGBA, GL_UNSIGNED_BYTE, img.rgba.as_ptr() as *const _);
        let f = if linear { GL_LINEAR } else { GL_NEAREST };
        for (p, v) in [(GL_TEXTURE_MIN_FILTER, f), (GL_TEXTURE_MAG_FILTER, f), (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE), (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE)] {
            glTexParameteri(GL_TEXTURE_2D, p, v);
        }
        t
    }
}

impl GlCanvas {
    /// `pano`: chỉ số 6 ảnh panorama (lọc tuyến tính cho mượt)
    pub fn new(imgs: &[Image], pano: Option<[usize; 6]>) -> Result<GlCanvas, String> {
        let prog = build_program(VS, FS, &["a_pos", "a_uv", "a_col"])?;
        let pprog = build_program(PVS, PFS, &["a_pos"])?;
        let cs = |s: &str| CString::new(s).unwrap();
        let (u_size, u_tex) = unsafe { (glGetUniformLocation(prog, cs("u_size").as_ptr()), glGetUniformLocation(prog, cs("u_tex").as_ptr())) };
        let mut vbo = 0;
        unsafe { glGenBuffers(1, &mut vbo) };
        let texs = imgs.iter().enumerate().map(|(i, im)| upload(im, pano.map_or(false, |p| p.contains(&i)))).collect();
        let white = upload(&Image { w: 1, h: 1, rgba: vec![255; 4] }, false);
        Ok(GlCanvas {
            prog,
            u_size,
            u_tex,
            pprog,
            vbo,
            texs,
            white,
            w: 1,
            h: 1,
            xf: Xf::ID,
            clip: None,
            verts: Vec::new(),
            cmds: Vec::new(),
            pano: None,
            diag: None,
            frames: 0,
            sizes: imgs.iter().map(|i| (i.w as f32, i.h as f32)).collect(),
            clear: true,
        })
    }

    pub fn begin(&mut self, w: i32, h: i32) {
        self.w = w.max(1);
        self.h = h.max(1);
        self.verts.clear();
        self.cmds.clear();
        self.pano = None;
        self.xf = Xf::ID;
        self.clip = None;
    }

    fn clip_i(&self) -> Option<[i32; 4]> {
        self.clip.map(|c| {
            let x0 = (c[0].round() as i32).clamp(0, self.w);
            let y0 = (c[1].round() as i32).clamp(0, self.h);
            let x1 = ((c[0] + c[2]).round() as i32).clamp(0, self.w);
            let y1 = ((c[1] + c[3]).round() as i32).clamp(0, self.h);
            [x0, y0, x1 - x0, y1 - y0]
        })
    }

    fn push(&mut self, tex: u32, dst: [f32; 4], uv: [f32; 4], col: u32) {
        let clip = self.clip_i();
        if self.cmds.last().map_or(true, |c| c.tex != tex || c.clip != clip) {
            self.cmds.push(Cmd { tex, clip, first: self.verts.len() as i32, count: 0 });
        }
        let c = [(col >> 24) as u8, (col >> 16) as u8, (col >> 8) as u8, col as u8];
        let p = |x: f32, y: f32, u: f32, v: f32| {
            let (px, py) = self.xf.apply(x, y);
            V { x: px, y: py, u, v, c }
        };
        let (x1, y1) = (dst[0] + dst[2], dst[1] + dst[3]);
        let (a, b, d, e) = (p(dst[0], dst[1], uv[0], uv[1]), p(x1, dst[1], uv[2], uv[1]), p(x1, y1, uv[2], uv[3]), p(dst[0], y1, uv[0], uv[3]));
        self.verts.extend_from_slice(&[a, b, d, a, d, e]);
        self.cmds.last_mut().unwrap().count += 6;
    }

    /// vẽ toàn bộ lệnh đã gom (panorama trước, rồi lô ảnh)
    pub fn end(&mut self) {
        unsafe {
            glViewport(0, 0, self.w, self.h);
            glDisable(GL_SCISSOR_TEST);
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_CULL_FACE);
            glDisable(GL_BLEND);
            if self.clear {
                glClearColor(0.1, 0.11, 0.13, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
            }
            if let Some((faces, yaw, pitch, fov, alpha, blur)) = self.pano {
                self.draw_pano(&faces, yaw, pitch, fov, alpha, blur);
                self.frames += 1;
                if self.frames == 40 {
                    let mut px = [0u8; 4];
                    glReadPixels(self.w / 2, self.h / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut _);
                    let e = glGetError();
                    self.diag = Some(format!("Panorama: điểm giữa màn hình = {},{},{},{} (glError {:#x}), {}x{}", px[0], px[1], px[2], px[3], e, self.w, self.h));
                }
            }
            if self.verts.is_empty() {
                return;
            }
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glUseProgram(self.prog);
            glUniform2f(self.u_size, self.w as f32, self.h as f32);
            glActiveTexture(GL_TEXTURE0);
            glUniform1i(self.u_tex, 0);
            glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
            glBufferData(GL_ARRAY_BUFFER, (self.verts.len() * std::mem::size_of::<V>()) as isize, self.verts.as_ptr() as *const _, GL_STREAM_DRAW);
            let stride = std::mem::size_of::<V>() as i32;
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, stride, 0 as *const _);
            glEnableVertexAttribArray(1);
            glVertexAttribPointer(1, 2, GL_FLOAT, 0, stride, 8 as *const _);
            glEnableVertexAttribArray(2);
            glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, 1, stride, 16 as *const _);
            let mut scissor = false;
            for c in &self.cmds {
                match c.clip {
                    Some(r) => {
                        if r[2] <= 0 || r[3] <= 0 {
                            continue;
                        }
                        if !scissor {
                            glEnable(GL_SCISSOR_TEST);
                            scissor = true;
                        }
                        glScissor(r[0], self.h - (r[1] + r[3]), r[2], r[3]);
                    }
                    None => {
                        if scissor {
                            glDisable(GL_SCISSOR_TEST);
                            scissor = false;
                        }
                    }
                }
                glBindTexture(GL_TEXTURE_2D, c.tex);
                glDrawArrays(GL_TRIANGLES, c.first, c.count);
            }
            glDisable(GL_SCISSOR_TEST);
            glDisable(GL_BLEND);
        }
    }

    unsafe fn draw_pano(&mut self, faces: &[usize; 6], yaw: f32, pitch: f32, fov: f32, alpha: f32, blur: f32) {
        let (y, p) = (yaw.to_radians(), pitch.to_radians());
        let f = [y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()];
        let up = [y.sin() * p.sin(), p.cos(), y.cos() * p.sin()];
        let r = [y.cos(), 0.0, -y.sin()];
        let t = (fov.to_radians() / 2.0).tan();
        let asp = self.w as f32 / self.h as f32;
        glUseProgram(self.pprog);
        let cs = |s: &str| CString::new(s).unwrap();
        glUniform3f(glGetUniformLocation(self.pprog, cs("u_f").as_ptr()), f[0], f[1], f[2]);
        glUniform3f(glGetUniformLocation(self.pprog, cs("u_r").as_ptr()), r[0], r[1], r[2]);
        glUniform3f(glGetUniformLocation(self.pprog, cs("u_u").as_ptr()), up[0], up[1], up[2]);
        glUniform2f(glGetUniformLocation(self.pprog, cs("u_k").as_ptr()), t * asp, t);
        glUniform1f(glGetUniformLocation(self.pprog, cs("u_a").as_ptr()), alpha);
        glUniform1i(glGetUniformLocation(self.pprog, cs("u_n").as_ptr()), if blur > 0.01 { 13 } else { 1 });
        glUniform2f(glGetUniformLocation(self.pprog, cs("u_b").as_ptr()), blur * 2.0 / self.w as f32, blur * 2.0 / self.h as f32);
        for (i, fi) in faces.iter().enumerate() {
            glActiveTexture(GL_TEXTURE0 + i as u32);
            glBindTexture(GL_TEXTURE_2D, self.texs[*fi]);
            glUniform1i(glGetUniformLocation(self.pprog, cs(&format!("t{i}")).as_ptr()), i as i32);
        }
        let quad: [f32; 12] = [-1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0];
        glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
        glBufferData(GL_ARRAY_BUFFER, 48, quad.as_ptr() as *const _, GL_STREAM_DRAW);
        glDisableVertexAttribArray(1);
        glDisableVertexAttribArray(2);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, null());
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glActiveTexture(GL_TEXTURE0);
    }
}

impl Canvas for GlCanvas {
    fn size(&self) -> (f32, f32) {
        (self.w as f32, self.h as f32)
    }
    fn fill(&mut self, r: [f32; 4], rgba: u32) {
        let w = self.white;
        self.push(w, r, [0.0, 0.0, 1.0, 1.0], rgba);
    }
    fn quad(&mut self, tex: usize, dst: [f32; 4], src: [f32; 4], rgba: u32) {
        let (iw, ih) = self.sizes[tex];
        let t = self.texs[tex];
        self.push(t, dst, [src[0] / iw, src[1] / ih, (src[0] + src[2]) / iw, (src[1] + src[3]) / ih], rgba);
    }
    fn set_xf(&mut self, xf: Xf) {
        self.xf = xf;
    }
    fn set_clip(&mut self, r: Option<[f32; 4]>) {
        self.clip = r;
    }
    fn panorama(&mut self, faces: &[usize; 6], yaw: f32, pitch: f32, fov: f32, alpha: f32, blur: f32) {
        self.pano = Some((*faces, yaw, pitch, fov, alpha, blur));
    }
    fn add_image(&mut self, w: usize, h: usize, rgba: &[u8]) -> usize {
        let t = upload(&Image { w, h, rgba: rgba.to_vec() }, false);
        self.texs.push(t);
        self.sizes.push((w as f32, h as f32));
        self.texs.len() - 1
    }
}
