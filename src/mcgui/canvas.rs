//! Mặt vẽ trừu tượng (GL trên máy thật, phần mềm khi kiểm tra) và lớp `Draw` vẽ theo đơn vị GUI của Minecraft.
use super::assets::{GuiAssets, Slice, Sprite};

/// Biến đổi affine 2D: x' = a*x + c*y + tx, y' = b*x + d*y + ty (dùng cho chữ splash xoay).
#[derive(Clone, Copy)]
pub struct Xf {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Xf {
    pub const ID: Xf = Xf { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }
}

pub trait Canvas {
    /// kích thước mặt vẽ theo điểm ảnh thật
    fn size(&self) -> (f32, f32);
    /// tô hình chữ nhật [x,y,w,h], màu RGBA
    fn fill(&mut self, r: [f32; 4], rgba: u32);
    /// vẽ ảnh `tex`, vùng nguồn `src` [x,y,w,h] tính bằng điểm ảnh của ảnh, đích `dst`, nhân màu `rgba`
    fn quad(&mut self, tex: usize, dst: [f32; 4], src: [f32; 4], rgba: u32);
    fn set_xf(&mut self, xf: Xf);
    fn set_clip(&mut self, r: Option<[f32; 4]>);
    /// panorama 6 mặt, camera xoay yaw/pitch (độ), góc nhìn dọc fov (độ), độ mờ alpha
    /// `blur`: bán kính làm mờ (điểm ảnh thật), 0 = không mờ
    fn panorama(&mut self, faces: &[usize; 6], yaw: f32, pitch: f32, fov: f32, alpha: f32, blur: f32);
    /// thêm một ảnh động (ví dụ icon server); trả về chỉ số dùng với `quad`, hoặc usize::MAX nếu không hỗ trợ
    fn add_image(&mut self, _w: usize, _h: usize, _rgba: &[u8]) -> usize {
        usize::MAX
    }
}

pub fn rgb(c: u32) -> u32 {
    (c << 8) | 0xff
}

/// Nhân thành phần RGB (bóng chữ = 0.25).
pub fn scale_rgb(c: u32, f: f32) -> u32 {
    let ch = |sh: u32| (((c >> sh) & 255) as f32 * f) as u32;
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | (c & 255)
}

pub fn with_alpha(c: u32, a: f32) -> u32 {
    (c & 0xffffff00) | ((((c & 255) as f32) * a.clamp(0.0, 1.0)).round() as u32)
}

/// Vẽ theo đơn vị GUI: mọi toạ độ nhân với hệ số phóng `k` (số nguyên như "GUI scale" của Minecraft).
pub struct Draw<'a> {
    pub c: &'a mut dyn Canvas,
    pub a: &'a GuiAssets,
    pub k: f32,
}

impl<'a> Draw<'a> {
    /// kích thước màn hình theo đơn vị GUI
    pub fn size(&self) -> (f32, f32) {
        let (w, h) = self.c.size();
        ((w / self.k).ceil(), (h / self.k).ceil())
    }

    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, rgba: u32) {
        let k = self.k;
        self.c.fill([x * k, y * k, w * k, h * k], rgba);
    }

    /// tô chuyển màu dọc (dải theo từng điểm ảnh GUI)
    pub fn gradient(&mut self, x: f32, y: f32, w: f32, h: f32, top: u32, bottom: u32) {
        let n = (h as usize).clamp(1, 96);
        let lerp = |a: u32, b: u32, t: f32| {
            let ch = |sh: u32| {
                let (p, q) = (((a >> sh) & 255) as f32, ((b >> sh) & 255) as f32);
                (p + (q - p) * t).round() as u32
            };
            (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
        };
        for i in 0..n {
            let (y0, y1) = (y + h * i as f32 / n as f32, y + h * (i + 1) as f32 / n as f32);
            self.fill(x, y0, w, y1 - y0, lerp(top, bottom, (i as f32 + 0.5) / n as f32));
        }
    }

    /// ảnh nguyên tấm hoặc một phần, kích thước đích theo đơn vị GUI
    pub fn blit(&mut self, tex: usize, dst: [f32; 4], src: [f32; 4], rgba: u32) {
        let k = self.k;
        self.c.quad(tex, [dst[0] * k, dst[1] * k, dst[2] * k, dst[3] * k], src, rgba);
    }

    /// ảnh có kích thước logic `lw` x `lh` (ví dụ logo 1024x256 coi như 256x64): `src` tính theo kích thước logic
    pub fn blit_logical(&mut self, tex: usize, dst: [f32; 4], src: [f32; 4], lw: f32, lh: f32, rgba: u32) {
        let (iw, ih) = (self.a.imgs[tex].w as f32, self.a.imgs[tex].h as f32);
        let (sx, sy) = (iw / lw, ih / lh);
        self.blit(tex, dst, [src[0] * sx, src[1] * sy, src[2] * sx, src[3] * sy], rgba);
    }

    /// vẽ một sprite GUI (có cắt 9 mảnh như Minecraft: phần giữa và cạnh được lát lại, không giãn)
    pub fn sprite(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32) {
        self.sprite_c(name, x, y, w, h, 0xffffffff);
    }

    pub fn sprite_c(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32, col: u32) {
        let Some(s) = self.a.sprites.get(name).copied() else { return };
        self.draw_sprite(&s, x, y, w, h, col);
    }

    pub fn has_sprite(&self, name: &str) -> bool {
        self.a.sprites.contains_key(name)
    }

    fn draw_sprite(&mut self, s: &Sprite, x: f32, y: f32, w: f32, h: f32, col: u32) {
        match s.slice {
            Slice::Stretch => self.blit(s.tex, [x, y, w, h], [0.0, 0.0, s.w, s.h], col),
            Slice::Tile => self.tile(s.tex, [x, y, w, h], [0.0, 0.0, s.w, s.h], col),
            Slice::Nine { l, t, r, b } => {
                // góc
                let (xm, ym) = (x + l, y + t);
                let (xr, yb) = (x + w - r, y + h - b);
                let (sw, sh) = (s.w, s.h);
                let tx = s.tex;
                self.blit(tx, [x, y, l, t], [0.0, 0.0, l, t], col);
                self.blit(tx, [xr, y, r, t], [sw - r, 0.0, r, t], col);
                self.blit(tx, [x, yb, l, b], [0.0, sh - b, l, b], col);
                self.blit(tx, [xr, yb, r, b], [sw - r, sh - b, r, b], col);
                // cạnh và giữa: lát
                self.tile(tx, [xm, y, xr - xm, t], [l, 0.0, sw - l - r, t], col);
                self.tile(tx, [xm, yb, xr - xm, b], [l, sh - b, sw - l - r, b], col);
                self.tile(tx, [x, ym, l, yb - ym], [0.0, t, l, sh - t - b], col);
                self.tile(tx, [xr, ym, r, yb - ym], [sw - r, t, r, sh - t - b], col);
                self.tile(tx, [xm, ym, xr - xm, yb - ym], [l, t, sw - l - r, sh - t - b], col);
            }
        }
    }

    /// lát vùng nguồn `src` (đơn vị điểm ảnh ảnh = đơn vị GUI) kín vùng đích, cắt mép cuối
    pub fn tile(&mut self, tex: usize, dst: [f32; 4], src: [f32; 4], col: u32) {
        if dst[2] <= 0.0 || dst[3] <= 0.0 || src[2] <= 0.0 || src[3] <= 0.0 {
            return;
        }
        let mut yy = 0.0;
        while yy < dst[3] {
            let hh = (dst[3] - yy).min(src[3]);
            let mut xx = 0.0;
            while xx < dst[2] {
                let ww = (dst[2] - xx).min(src[2]);
                self.blit(tex, [dst[0] + xx, dst[1] + yy, ww, hh], [src[0], src[1], ww, hh], col);
                xx += src[2];
            }
            yy += src[3];
        }
    }

    pub fn text_width(&self, s: &str) -> f32 {
        self.a.font.width(s)
    }

    /// vẽ chữ tại (x, y) là góc trên-trái; trả về x sau ký tự cuối
    pub fn text_raw(&mut self, s: &str, x: f32, y: f32, color: u32) -> f32 {
        let mut cx = x;
        let mut col = color;
        let mut it = s.chars();
        while let Some(c) = it.next() {
            if c == '§' {
                if let Some(code) = it.next() {
                    col = match code.to_ascii_lowercase() {
                        '0' => 0x000000ff,
                        '1' => 0x0000aaff,
                        '2' => 0x00aa00ff,
                        '3' => 0x00aaaaff,
                        '4' => 0xaa0000ff,
                        '5' => 0xaa00aaff,
                        '6' => 0xffaa00ff,
                        '7' => 0xaaaaaaff,
                        '8' => 0x555555ff,
                        '9' => 0x5555ffff,
                        'a' => 0x55ff55ff,
                        'b' => 0x55ffffff,
                        'c' => 0xff5555ff,
                        'd' => 0xff55ffff,
                        'e' => 0xffff55ff,
                        'f' => 0xffffffff,
                        'r' => color,
                        _ => col,
                    };
                    col = (col & 0xffffff00) | (color & 255);
                }
                continue;
            }
            let Some(g) = self.a.font.glyph(c).copied() else { continue };
            if g.w > 0.0 {
                self.blit(g.tex, [cx, y + g.yoff, g.dw, g.dh], [g.u, g.v, g.w, g.h], col);
            }
            cx += g.adv;
        }
        cx
    }

    /// chữ có bóng (màu * 0.25 lệch 1 đơn vị) như vanilla
    pub fn text(&mut self, s: &str, x: f32, y: f32, color: u32) -> f32 {
        self.text_raw(s, x + 1.0, y + 1.0, scale_rgb(color, 0.25));
        self.text_raw(s, x, y, color)
    }

    pub fn text_centered(&mut self, s: &str, cx: f32, y: f32, color: u32) {
        let w = self.text_width(s);
        self.text(s, cx - w / 2.0, y, color);
    }

    pub fn text_right(&mut self, s: &str, right: f32, y: f32, color: u32) {
        let w = self.text_width(s);
        self.text(s, right - w, y, color);
    }

    /// nền đất/đá của menu: ảnh 16x16 lát ở cỡ 32 đơn vị, tối đi theo màu `tint`
    pub fn tiled_bg(&mut self, tex: Option<usize>, r: [f32; 4], tint: u32) {
        let Some(t) = tex else { return };
        let (iw, ih) = (self.a.imgs[t].w as f32, self.a.imgs[t].h as f32);
        let tile = 32.0;
        let mut y = 0.0;
        while y < r[3] {
            let hh = (r[3] - y).min(tile);
            let mut x = 0.0;
            while x < r[2] {
                let ww = (r[2] - x).min(tile);
                self.blit(t, [r[0] + x, r[1] + y, ww, hh], [0.0, 0.0, iw * ww / tile, ih * hh / tile], tint);
                x += tile;
            }
            y += tile;
        }
    }
}
