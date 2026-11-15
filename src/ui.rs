//! Giao diện 2D: nút, thanh vật phẩm, tâm ngắm. Chỉ dùng thư viện chuẩn.
//! Mọi toạ độ vào/ra là pixel màn hình, góc trên trái là (0, 0).

use crate::world::{block_color, HOTBAR, HOTBAR_SLOTS};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UiVertex {
    pub pos: [f32; 2],   // toạ độ NDC (-1..1)
    pub color: [f32; 4], // RGBA, RGB đã ở dạng tuyến tính
}

/// Số đỉnh tối đa gpu.rs dành cho giao diện mỗi khung hình.
pub const UI_MAX_VERTICES: usize = 2048;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub screen: (f32, f32),
    /// Cạnh ngắn của màn hình, làm đơn vị để mọi thứ co giãn theo máy.
    pub unit: f32,
    pub hotbar: [Rect; HOTBAR_SLOTS],
    pub jump: Rect,
    pub fly: Rect,
}

impl Layout {
    pub fn new(w: f32, h: f32) -> Self {
        let u = w.min(h).max(1.0);
        // Chừa chỗ cho thanh điều hướng của Android ở dưới và thanh trạng thái ở trên.
        let bottom = 0.15 * u;
        let top = 0.14 * u;

        let slot = 0.125 * u;
        let gap = 0.012 * u;
        let total = HOTBAR_SLOTS as f32 * slot + (HOTBAR_SLOTS as f32 - 1.0) * gap;
        let hx = (w - total) * 0.5;
        let hy = h - bottom - slot;
        let mut hotbar = [Rect { x: 0.0, y: 0.0, w: slot, h: slot }; HOTBAR_SLOTS];
        for (i, r) in hotbar.iter_mut().enumerate() {
            r.x = hx + i as f32 * (slot + gap);
            r.y = hy;
        }

        let j = 0.18 * u;
        let jump = Rect { x: w - 0.05 * u - j, y: hy - 0.04 * u - j, w: j, h: j };
        let f = 0.11 * u;
        let fly = Rect { x: w - 0.05 * u - f, y: top, w: f, h: f };

        Self { screen: (w, h), unit: u, hotbar, jump, fly }
    }
}

// ---------- Vẽ ----------

/// Màu sRGB -> tuyến tính (bề mặt hiển thị là sRGB), giữ nguyên alpha.
fn rgba(rgb: [f32; 3], a: f32) -> [f32; 4] {
    [rgb[0] * rgb[0], rgb[1] * rgb[1], rgb[2] * rgb[2], a]
}

struct Painter {
    w: f32,
    h: f32,
    v: Vec<UiVertex>,
}

impl Painter {
    fn ndc(&self, x: f32, y: f32) -> [f32; 2] {
        [x / self.w * 2.0 - 1.0, 1.0 - y / self.h * 2.0]
    }

    fn tri(&mut self, a: (f32, f32), b: (f32, f32), c: (f32, f32), color: [f32; 4]) {
        for p in [a, b, c] {
            let pos = self.ndc(p.0, p.1);
            self.v.push(UiVertex { pos, color });
        }
    }

    fn rect(&mut self, r: Rect, color: [f32; 4]) {
        let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
        self.tri((x0, y0), (x1, y0), (x1, y1), color);
        self.tri((x0, y0), (x1, y1), (x0, y1), color);
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: [f32; 4]) {
        const N: usize = 20;
        for i in 0..N {
            let a0 = i as f32 / N as f32 * std::f32::consts::TAU;
            let a1 = (i + 1) as f32 / N as f32 * std::f32::consts::TAU;
            self.tri(
                (cx, cy),
                (cx + r * a0.cos(), cy + r * a0.sin()),
                (cx + r * a1.cos(), cy + r * a1.sin()),
                color,
            );
        }
    }

    /// Tam giác hướng lên, nằm gọn trong hình chữ nhật.
    fn arrow_up(&mut self, r: Rect, color: [f32; 4]) {
        self.tri(
            (r.x + r.w * 0.5, r.y),
            (r.x + r.w, r.y + r.h),
            (r.x, r.y + r.h),
            color,
        );
    }
}

fn inset(r: Rect, k: f32) -> Rect {
    Rect { x: r.x + k, y: r.y + k, w: r.w - 2.0 * k, h: r.h - 2.0 * k }
}

/// Dựng toàn bộ giao diện của một khung hình.
/// `stick` là (điểm chạm đầu, điểm chạm hiện tại) của cần điều khiển, nếu đang dùng.
pub fn build(
    layout: &Layout,
    selected: usize,
    flying: bool,
    jump_pressed: bool,
    stick: Option<([f32; 2], [f32; 2])>,
) -> Vec<UiVertex> {
    let (w, h) = layout.screen;
    let u = layout.unit;
    let mut p = Painter { w, h, v: Vec::with_capacity(512) };

    // Cần điều khiển.
    if let Some((o, c)) = stick {
        let base = 0.2 * u;
        let (mut dx, mut dy) = (c[0] - o[0], c[1] - o[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len > base {
            dx *= base / len;
            dy *= base / len;
        }
        p.circle(o[0], o[1], base, rgba([1.0, 1.0, 1.0], 0.12));
        p.circle(o[0] + dx, o[1] + dy, 0.07 * u, rgba([1.0, 1.0, 1.0], 0.40));
    }

    // Tâm ngắm.
    let (cx, cy) = (w * 0.5, h * 0.5);
    let arm = 0.028 * u;
    let thick = (0.005 * u).max(2.0);
    let cross = rgba([1.0, 1.0, 1.0], 0.85);
    p.rect(Rect { x: cx - arm, y: cy - thick * 0.5, w: arm * 2.0, h: thick }, cross);
    p.rect(Rect { x: cx - thick * 0.5, y: cy - arm, w: thick, h: arm * 2.0 }, cross);

    // Nút nhảy.
    let j = layout.jump;
    let jbg = if jump_pressed { 0.65 } else { 0.40 };
    p.rect(j, rgba([0.0, 0.0, 0.0], jbg));
    p.arrow_up(inset(j, j.w * 0.25), rgba([1.0, 1.0, 1.0], 0.9));

    // Nút bay: hai mũi tên chồng nhau, xanh khi đang bật.
    let f = layout.fly;
    p.rect(f, rgba([0.0, 0.0, 0.0], 0.40));
    let tint = if flying { [0.45, 1.0, 0.45] } else { [1.0, 1.0, 1.0] };
    let half = f.h * 0.28;
    let pad = f.w * 0.22;
    p.arrow_up(Rect { x: f.x + pad, y: f.y + f.h * 0.14, w: f.w - 2.0 * pad, h: half }, rgba(tint, 0.9));
    p.arrow_up(Rect { x: f.x + pad, y: f.y + f.h * 0.52, w: f.w - 2.0 * pad, h: half }, rgba(tint, 0.9));

    // Thanh vật phẩm.
    for (i, r) in layout.hotbar.iter().enumerate() {
        let border = if i == selected { 0.95 } else { 0.30 };
        p.rect(*r, rgba([1.0, 1.0, 1.0], border));
        p.rect(inset(*r, 0.07 * r.w), rgba([0.0, 0.0, 0.0], 0.60));
        p.rect(inset(*r, 0.22 * r.w), rgba(block_color(HOTBAR[i]), 1.0));
    }

    p.v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    fn inside(r: &Rect, w: f32, h: f32) -> bool {
        r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= w && r.y + r.h <= h
    }

    #[test]
    fn layout_fits_and_does_not_overlap() {
        for (w, h) in [(720.0, 1600.0), (1600.0, 720.0), (2000.0, 1000.0), (1080.0, 2400.0)] {
            let l = Layout::new(w, h);
            assert!(inside(&l.jump, w, h) && inside(&l.fly, w, h), "{w}x{h}");
            for r in &l.hotbar {
                assert!(inside(r, w, h), "{w}x{h} thanh vật phẩm ra ngoài");
                assert!(!overlap(r, &l.jump), "{w}x{h} nút nhảy đè thanh vật phẩm");
            }
            assert!(!overlap(&l.jump, &l.fly), "{w}x{h}");
            // Các ô liền kề không đè nhau.
            for pair in l.hotbar.windows(2) {
                assert!(!overlap(&pair[0], &pair[1]));
            }
            // Tâm màn hình phải trống để chạm xoay camera và nhắm khối.
            let (cx, cy) = (w * 0.5, h * 0.5);
            assert!(!l.jump.contains(cx, cy) && !l.fly.contains(cx, cy));
            assert!(l.hotbar.iter().all(|r| !r.contains(cx, cy)));
        }
    }

    #[test]
    fn build_is_sane() {
        let l = Layout::new(720.0, 1600.0);
        for stick in [None, Some(([200.0, 1000.0], [260.0, 900.0])), Some(([200.0, 1000.0], [900.0, 100.0]))] {
            let v = build(&l, 2, true, true, stick);
            assert!(!v.is_empty());
            assert_eq!(v.len() % 3, 0);
            assert!(v.len() <= UI_MAX_VERTICES, "{} đỉnh", v.len());
            for vert in &v {
                assert!((0.0..=1.0).contains(&vert.color[3]));
                assert!(vert.color[..3].iter().all(|c| (0.0..=1.0).contains(c)));
            }
        }
        // Không có cần điều khiển thì mọi đỉnh nằm trong màn hình.
        for vert in build(&l, 0, false, false, None) {
            assert!(vert.pos[0].abs() <= 1.0001 && vert.pos[1].abs() <= 1.0001, "{:?}", vert.pos);
        }
    }

    #[test]
    fn ndc_orientation() {
        // Góc trên trái của màn hình là (-1, +1) trong NDC.
        let p = Painter { w: 100.0, h: 200.0, v: Vec::new() };
        assert_eq!(p.ndc(0.0, 0.0), [-1.0, 1.0]);
        assert_eq!(p.ndc(100.0, 200.0), [1.0, -1.0]);
        assert_eq!(p.ndc(50.0, 100.0), [0.0, 0.0]);
    }
}
