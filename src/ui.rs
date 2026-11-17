//! Bảng màu và hàm vẽ dùng chung cho giao diện launcher (phẳng, tối, nhấn màu ngọc).
use crate::gfx::Gfx;

pub const BG: u32 = 0x0d1117ff;
pub const BAR: u32 = 0x111821ff;
pub const CARD: u32 = 0x18212cff;
pub const CARD_HI: u32 = 0x233245ff;
pub const LINE: u32 = 0x263447ff;
pub const TEXT: u32 = 0xe6edf3ff;
pub const DIM: u32 = 0x8b9bb0ff;
pub const ACCENT: u32 = 0x2dd4bfff;
pub const ACCENT_DK: u32 = 0x14917fff;
pub const WARN: u32 = 0xf59e0bff;
pub const OFF: u32 = 0x232c38ff;

#[derive(Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn has(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }
}

pub fn hit(r: Rect, tap: Option<(f32, f32)>) -> bool {
    tap.map_or(false, |t| r.has(t))
}

fn lerp(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let x = ((a >> sh) & 255) as f32;
        let y = ((b >> sh) & 255) as f32;
        (x + (y - x) * t).round() as u32
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Tô chuyển màu theo chiều dọc (16 dải).
pub fn vgrad(g: &mut Gfx, r: Rect, top: u32, bottom: u32) {
    let n = 16;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let y0 = r.y + r.h * i as f32 / n as f32;
        let y1 = r.y + r.h * (i + 1) as f32 / n as f32;
        g.rect(r.x, y0, r.w, y1 - y0 + 0.5, lerp(top, bottom, t));
    }
}

/// Chữ căn giữa trong hình chữ nhật.
pub fn text_c(g: &mut Gfx, r: Rect, size: f32, color: u32, s: &str) {
    let tw = g.text_width(s, size);
    let lh = g.line_height(size);
    g.text(r.x + (r.w - tw) / 2.0, r.y + (r.h - lh) / 2.0, size, color, s);
}

/// Chữ căn giữa theo chiều dọc, bắt đầu từ x.
pub fn text_v(g: &mut Gfx, x: f32, r: Rect, size: f32, color: u32, s: &str) {
    let lh = g.line_height(size);
    g.text(x, r.y + (r.h - lh) / 2.0, size, color, s);
}

// Icon pixel 8x8, bit cao là cột trái.
pub const ICON_USER: [u8; 8] = [0b00111100, 0b01111110, 0b01111110, 0b00111100, 0b00000000, 0b01111110, 0b11111111, 0b11111111];
pub const ICON_CUBE: [u8; 8] = [0b01111110, 0b11000011, 0b10111101, 0b10100101, 0b10100101, 0b10111101, 0b11000011, 0b01111110];
pub const ICON_LOG: [u8; 8] = [0b11111111, 0b00000000, 0b11111111, 0b00000000, 0b11111111, 0b00000000, 0b11111000, 0b00000000];
pub const ICON_BACK: [u8; 8] = [0b00001100, 0b00011000, 0b00110000, 0b01100000, 0b01100000, 0b00110000, 0b00011000, 0b00001100];
pub const ICON_UP: [u8; 8] = [0b00000000, 0b00011000, 0b00111100, 0b01100110, 0b11000011, 0b10000001, 0b00000000, 0b00000000];
pub const ICON_TRASH: [u8; 8] = [0b00111100, 0b11111111, 0b01111110, 0b01011010, 0b01011010, 0b01011010, 0b01111110, 0b00000000];
pub const ICON_CHECK: [u8; 8] = [0b00000001, 0b00000011, 0b00000110, 0b10001100, 0b11011000, 0b01110000, 0b00100000, 0b00000000];

/// Vẽ icon pixel; `px` là cạnh mỗi ô.
pub fn icon(g: &mut Gfx, bits: &[u8; 8], x: f32, y: f32, px: f32, color: u32) {
    for (r, &row) in bits.iter().enumerate() {
        let mut c = 0usize;
        while c < 8 {
            if row & (0x80u8 >> c) != 0 {
                let start = c;
                while c < 8 && row & (0x80u8 >> c) != 0 {
                    c += 1;
                }
                g.rect(x + start as f32 * px, y + r as f32 * px, (c - start) as f32 * px, px, color);
            } else {
                c += 1;
            }
        }
    }
}

const PALETTE: [u32; 6] = [0x2dd4bf, 0xf59e0b, 0x60a5fa, 0xf472b6, 0xa78bfa, 0x34d399];

/// Ô chữ cái đầu của tên, màu theo tên.
pub fn avatar(g: &mut Gfx, r: Rect, name: &str) {
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    g.rect(r.x, r.y, r.w, r.h, (PALETTE[(h % 6) as usize] << 8) | 0xff);
    let init = name.chars().next().map_or('?', |c| c.to_ascii_uppercase()).to_string();
    text_c(g, r, r.h * 0.5, 0x06141aff, &init);
}
