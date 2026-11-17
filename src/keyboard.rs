//! Bàn phím vẽ tay (chữ, số, ký tự đặc biệt) để nhập tên, email, mật khẩu; không phụ thuộc bàn phím hệ thống.
use crate::{
    gfx::Gfx,
    ui::{hit, text_c, Rect, ACCENT_DK, CARD_HI, TEXT},
};

#[derive(Clone, Copy, PartialEq)]
pub enum Kb {
    Ch(char),
    Shift,
    Sym,
    Back,
    Done,
    Cancel,
}

pub struct Keyboard {
    shift: bool,
    sym: bool,
}

impl Keyboard {
    pub fn new() -> Keyboard {
        Keyboard { shift: false, sym: false }
    }

    pub fn reset(&mut self) {
        self.shift = false;
        self.sym = false;
    }

    /// Vẽ bàn phím trong `area`; trả về phím vừa chạm (Shift và ?123 tự xử lý bên trong).
    pub fn draw(&mut self, g: &mut Gfx, area: Rect, tap: Option<(f32, f32)>) -> Option<Kb> {
        let (sh, sy) = (self.shift, self.sym);
        let ch = |c: char| {
            let c = if sh { c.to_ascii_uppercase() } else { c };
            (c.to_string(), 1.0f32, Kb::Ch(c))
        };
        let plain = |c: char| (c.to_string(), 1.0f32, Kb::Ch(c));
        let row = |s: &str| s.chars().map(|c| ch(c)).collect::<Vec<_>>();
        let prow = |s: &str| s.chars().map(|c| plain(c)).collect::<Vec<_>>();
        let mut rows: Vec<Vec<(String, f32, Kb)>> = vec![prow("1234567890")];
        if !sy {
            rows.push(row("qwertyuiop"));
            rows.push(row("asdfghjkl"));
            let mut r3 = vec![("SHIFT".to_string(), 1.6f32, Kb::Shift)];
            r3.extend(row("zxcvbnm"));
            r3.push(("XÓA".to_string(), 1.6, Kb::Back));
            rows.push(r3);
        } else {
            rows.push(prow("@#$%&-+()/"));
            rows.push(prow("*'\":;!?,.="));
            let mut r3 = prow("~^<>[]{}|");
            r3.push(("XÓA".to_string(), 1.6, Kb::Back));
            rows.push(r3);
        }
        rows.push(vec![
            (if sy { "ABC" } else { "?123" }.to_string(), 1.8, Kb::Sym),
            plain('.'),
            plain('@'),
            plain('_'),
            plain('-'),
            ("HỦY".to_string(), 1.8, Kb::Cancel),
            ("XONG".to_string(), 2.4, Kb::Done),
        ]);

        let s = g.scale;
        let gap = 6.0 * s;
        let kh = (area.h - gap * (rows.len() as f32 + 1.0)) / rows.len() as f32;
        let mut out: Option<Kb> = None;
        for (ri, r) in rows.iter().enumerate() {
            let total: f32 = r.iter().map(|k| k.1).sum();
            let avail = area.w - gap * (r.len() as f32 + 1.0);
            let mut x = area.x + gap;
            let y = area.y + gap + ri as f32 * (kh + gap);
            for (label, wt, code) in r {
                let kw = avail * wt / total;
                let rc = Rect::new(x, y, kw, kh);
                let col = if *code == Kb::Done || (*code == Kb::Shift && self.shift) { ACCENT_DK } else { CARD_HI };
                g.rect(rc.x, rc.y, rc.w, rc.h, col);
                let size = if label.chars().count() > 1 { 13.0 * s } else { 18.0 * s };
                text_c(g, rc, size, TEXT, label);
                if hit(rc, tap) {
                    match *code {
                        Kb::Shift => self.shift = !self.shift,
                        Kb::Sym => self.sym = !self.sym,
                        c => out = Some(c),
                    }
                }
                x += kw + gap;
            }
        }
        if let Some(Kb::Ch(_)) = out {
            self.shift = false;
        }
        out
    }
}
