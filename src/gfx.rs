//! Vẽ giao diện bằng GLES: hình chữ nhật + chữ (fontdue rasterize, atlas một kênh). Tất cả gộp trong một VBO.
use std::{collections::HashMap, ffi::CString, ptr::null};

use fontdue::{Font, FontSettings};

use crate::gles::*;

const ATLAS: i32 = 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vtx {
    x: f32,
    y: f32,
    u: f32,
    v: f32,
    c: [u8; 4],
}

#[derive(Clone, Copy)]
struct Glyph {
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
    w: f32,
    h: f32,
    xmin: f32,
    ymin: f32,
    adv: f32,
}

struct Cmd {
    clip: [i32; 4],
    first: i32,
    count: i32,
}

pub fn rgba(c: u32) -> [u8; 4] {
    [(c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8]
}

pub struct Gfx {
    prog: u32,
    vbo: u32,
    tex: u32,
    u_size: i32,
    u_tex: i32,
    pub w: i32,
    pub h: i32,
    /// Hệ số theo mật độ màn hình (1.0 = 360 px rộng).
    pub scale: f32,
    font: Option<Font>,
    cur_x: i32,
    cur_y: i32,
    row_h: i32,
    glyphs: HashMap<(char, u32), Glyph>,
    verts: Vec<Vtx>,
    cmds: Vec<Cmd>,
    clip: [i32; 4],
}

fn compile(kind: u32, src: &str) -> Result<u32, String> {
    unsafe {
        let s = glCreateShader(kind);
        let c = CString::new(src).unwrap();
        let p = c.as_ptr();
        glShaderSource(s, 1, &p, null());
        glCompileShader(s);
        let mut ok = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut buf = vec![0u8; 1024];
            let mut n = 0;
            glGetShaderInfoLog(s, 1024, &mut n, buf.as_mut_ptr() as *mut _);
            return Err(format!("shader lỗi: {}", String::from_utf8_lossy(&buf[..n.max(0) as usize])));
        }
        Ok(s)
    }
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
void main(){ float a = texture(u_tex, v_uv).r; o = vec4(v_col.rgb, v_col.a * a); }";

impl Gfx {
    /// Cần context GL đang hiện hành.
    pub fn new() -> Result<Gfx, String> {
        unsafe {
            let vs = compile(GL_VERTEX_SHADER, VS)?;
            let fs = compile(GL_FRAGMENT_SHADER, FS)?;
            let prog = glCreateProgram();
            glAttachShader(prog, vs);
            glAttachShader(prog, fs);
            for (i, n) in ["a_pos", "a_uv", "a_col"].iter().enumerate() {
                let c = CString::new(*n).unwrap();
                glBindAttribLocation(prog, i as u32, c.as_ptr());
            }
            glLinkProgram(prog);
            let mut ok = 0;
            glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
            if ok == 0 {
                let mut buf = vec![0u8; 1024];
                let mut n = 0;
                glGetProgramInfoLog(prog, 1024, &mut n, buf.as_mut_ptr() as *mut _);
                return Err(format!("link lỗi: {}", String::from_utf8_lossy(&buf[..n.max(0) as usize])));
            }
            let cs = |s: &str| CString::new(s).unwrap();
            let u_size = glGetUniformLocation(prog, cs("u_size").as_ptr());
            let u_tex = glGetUniformLocation(prog, cs("u_tex").as_ptr());

            let mut vbo = 0;
            glGenBuffers(1, &mut vbo);
            let mut tex = 0;
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_R8, ATLAS, ATLAS, 0, GL_RED, GL_UNSIGNED_BYTE, null());
            for (p, v) in [
                (GL_TEXTURE_MIN_FILTER, GL_LINEAR),
                (GL_TEXTURE_MAG_FILTER, GL_LINEAR),
                (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
                (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
            ] {
                glTexParameteri(GL_TEXTURE_2D, p, v);
            }
            // Ô trắng 2x2 ở góc atlas để vẽ hình chữ nhật đặc.
            let white = [255u8; 4];
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RED, GL_UNSIGNED_BYTE, white.as_ptr() as *const _);

            let mut font = None;
            for p in [
                "/system/fonts/Roboto-Regular.ttf",
                "/system/fonts/NotoSans-Regular.ttf",
                "/system/fonts/DroidSans.ttf",
            ] {
                if let Ok(b) = std::fs::read(p) {
                    if let Ok(f) = Font::from_bytes(b, FontSettings::default()) {
                        font = Some(f);
                        break;
                    }
                }
            }

            Ok(Gfx {
                prog,
                vbo,
                tex,
                u_size,
                u_tex,
                w: 1,
                h: 1,
                scale: 1.0,
                font,
                cur_x: 3,
                cur_y: 0,
                row_h: 0,
                glyphs: HashMap::new(),
                verts: Vec::new(),
                cmds: Vec::new(),
                clip: [0, 0, 1, 1],
            })
        }
    }

    pub fn has_font(&self) -> bool {
        self.font.is_some()
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w.max(1);
        self.h = h.max(1);
        self.scale = (self.w.min(self.h) as f32 / 360.0).max(1.0);
    }

    pub fn begin(&mut self) {
        self.verts.clear();
        self.cmds.clear();
        self.clip = [0, 0, self.w, self.h];
    }

    /// Giới hạn vùng vẽ (pixel, gốc trên-trái).
    pub fn set_clip(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let x0 = (x as i32).clamp(0, self.w);
        let y0 = (y as i32).clamp(0, self.h);
        let x1 = ((x + w) as i32).clamp(0, self.w);
        let y1 = ((y + h) as i32).clamp(0, self.h);
        self.clip = [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)];
    }

    pub fn clear_clip(&mut self) {
        self.clip = [0, 0, self.w, self.h];
    }

    fn quad(&mut self, x: f32, y: f32, w: f32, h: f32, uv: [f32; 4], c: [u8; 4]) {
        if self.cmds.last().map_or(true, |m| m.clip != self.clip) {
            self.cmds.push(Cmd { clip: self.clip, first: self.verts.len() as i32, count: 0 });
        }
        let (x1, y1) = (x + w, y + h);
        let v = |x, y, u, v| Vtx { x, y, u, v, c };
        let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
        self.verts.extend_from_slice(&[v(x, y, u0, v0), v(x1, y, u1, v0), v(x1, y1, u1, v1), v(x, y, u0, v0), v(x1, y1, u1, v1), v(x, y1, u0, v1)]);
        self.cmds.last_mut().unwrap().count += 6;
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let t = 1.0 / ATLAS as f32;
        self.quad(x, y, w, h, [t, t, t, t], rgba(color));
    }

    fn glyph(&mut self, ch: char, px: u32) -> Option<Glyph> {
        if let Some(g) = self.glyphs.get(&(ch, px)) {
            return Some(*g);
        }
        let font = self.font.as_ref()?;
        let (m, bitmap) = font.rasterize(ch, px as f32);
        let (gw, gh) = (m.width as i32, m.height as i32);
        let mut g = Glyph { u0: 0.0, v0: 0.0, u1: 0.0, v1: 0.0, w: gw as f32, h: gh as f32, xmin: m.xmin as f32, ymin: m.ymin as f32, adv: m.advance_width };
        if gw > 0 && gh > 0 {
            if self.cur_x + gw + 1 > ATLAS {
                self.cur_x = 0;
                self.cur_y += self.row_h + 1;
                self.row_h = 0;
            }
            if self.cur_y + gh > ATLAS {
                // Atlas đầy: bỏ glyph này (chỉ xảy ra khi dùng quá nhiều cỡ chữ).
                g.w = 0.0;
                g.h = 0.0;
                self.glyphs.insert((ch, px), g);
                return Some(g);
            }
            unsafe {
                glBindTexture(GL_TEXTURE_2D, self.tex);
                glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
                glTexSubImage2D(GL_TEXTURE_2D, 0, self.cur_x, self.cur_y, gw, gh, GL_RED, GL_UNSIGNED_BYTE, bitmap.as_ptr() as *const _);
            }
            let a = ATLAS as f32;
            g.u0 = self.cur_x as f32 / a;
            g.v0 = self.cur_y as f32 / a;
            g.u1 = (self.cur_x + gw) as f32 / a;
            g.v1 = (self.cur_y + gh) as f32 / a;
            self.cur_x += gw + 1;
            self.row_h = self.row_h.max(gh);
        }
        self.glyphs.insert((ch, px), g);
        Some(g)
    }

    fn px(size: f32) -> u32 {
        size.round().max(6.0) as u32
    }

    pub fn line_height(&self, size: f32) -> f32 {
        self.font
            .as_ref()
            .and_then(|f| f.horizontal_line_metrics(Self::px(size) as f32))
            .map_or(size * 1.3, |m| m.new_line_size)
    }

    pub fn text_width(&mut self, s: &str, size: f32) -> f32 {
        let px = Self::px(size);
        s.chars().filter_map(|c| self.glyph(c, px)).map(|g| g.adv).sum()
    }

    /// Vẽ chữ, (x, y) là góc trên-trái của dòng. Trả về chiều rộng.
    pub fn text(&mut self, x: f32, y: f32, size: f32, color: u32, s: &str) -> f32 {
        let px = Self::px(size);
        let ascent = self
            .font
            .as_ref()
            .and_then(|f| f.horizontal_line_metrics(px as f32))
            .map_or(size, |m| m.ascent);
        let base = (y + ascent).round();
        let col = rgba(color);
        let mut pen = x;
        for ch in s.chars() {
            let Some(g) = self.glyph(ch, px) else { continue };
            if g.w > 0.0 {
                let gx = (pen + g.xmin).round();
                let gy = base - g.ymin - g.h;
                self.quad(gx, gy, g.w, g.h, [g.u0, g.v0, g.u1, g.v1], col);
            }
            pen += g.adv;
        }
        pen - x
    }

    /// Ngắt dòng theo chiều rộng tối đa; ưu tiên ngắt ở khoảng trắng, không có thì ngắt giữa từ.
    pub fn wrap(&mut self, s: &str, size: f32, max_w: f32) -> Vec<String> {
        let px = Self::px(size);
        let mut out = Vec::new();
        let mut line = String::new();
        let mut w = 0.0f32;
        let mut last_space: Option<usize> = None; // chỉ số byte trong `line`
        for ch in s.chars() {
            let adv = self.glyph(ch, px).map_or(0.0, |g| g.adv);
            if w + adv > max_w && !line.is_empty() {
                match last_space {
                    Some(i) => {
                        let rest = line[i + 1..].to_string();
                        line.truncate(i);
                        out.push(std::mem::take(&mut line));
                        line = rest;
                    }
                    None => out.push(std::mem::take(&mut line)),
                }
                w = line.chars().map(|c| self.glyph(c, px).map_or(0.0, |g| g.adv)).sum();
                last_space = None;
            }
            if ch == ' ' {
                last_space = Some(line.len());
            }
            line.push(ch);
            w += adv;
        }
        out.push(line);
        out
    }

    /// Xả mọi thứ đã gom ra màn hình (chưa swap).
    pub fn end(&mut self, bg: u32) {
        unsafe {
            glViewport(0, 0, self.w, self.h);
            glDisable(GL_SCISSOR_TEST);
            let c = rgba(bg);
            glClearColor(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            if self.verts.is_empty() {
                return;
            }
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glEnable(GL_SCISSOR_TEST);
            glUseProgram(self.prog);
            glUniform2f(self.u_size, self.w as f32, self.h as f32);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_2D, self.tex);
            glUniform1i(self.u_tex, 0);
            glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
            glBufferData(GL_ARRAY_BUFFER, (self.verts.len() * std::mem::size_of::<Vtx>()) as isize, self.verts.as_ptr() as *const _, GL_STREAM_DRAW);
            let stride = std::mem::size_of::<Vtx>() as i32;
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 2, GL_FLOAT, 0, stride, 0 as *const _);
            glEnableVertexAttribArray(1);
            glVertexAttribPointer(1, 2, GL_FLOAT, 0, stride, 8 as *const _);
            glEnableVertexAttribArray(2);
            glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, 1, stride, 16 as *const _);
            for c in &self.cmds {
                if c.clip[2] <= 0 || c.clip[3] <= 0 {
                    continue;
                }
                glScissor(c.clip[0], self.h - (c.clip[1] + c.clip[3]), c.clip[2], c.clip[3]);
                glDrawArrays(GL_TRIANGLES, c.first, c.count);
            }
        }
    }
}
