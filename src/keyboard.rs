//! Bàn phím chữ-số vẽ tay (nhập tên tài khoản), không phụ thuộc bàn phím hệ thống.
use crate::{
    gfx::Gfx,
    ui::{hit, text_c, Rect, ACCENT_DK, CARD_HI, TEXT},
};

#[derive(Clone, Copy, PartialEq)]
pub enum Kb {
    Ch(char),
    Shift,
    Back,
    Done,
    Cancel,
}

pub struct Keyboard {
    shift: bool,
}

impl Keyboard {
    pub fn new() -> Keyboard {
        Keyboard { shift: false }
    }

    pub fn reset(&mut self) {
        self.shift = false;
    }

    /// Vẽ bàn phím trong `area`; trả về phím vừa chạm (Shift tự xử lý bên trong).
    pub fn draw(&mut self, g: &mut Gfx, area: Rect, tap: Option<(f32, f32)>) -> Option<Kb> {
        let sh = self.shift;
        let ch = |c: char| {
            let c = if sh { c.to_ascii_uppercase() } else { c };
            (c.to_string(), 1.0f32, Kb::Ch(c))
        };
        let row = |s: &str| s.chars().map(|c| ch(c)).collect::<Vec<_>>();
        let mut rows: Vec<Vec<(String, f32, Kb)>> = vec![row("1234567890"), row("qwertyuiop"), row("asdfghjkl")];
        let mut r3 = vec![("SHIFT".to_string(), 1.6f32, Kb::Shift)];
        r3.extend(row("zxcvbnm"));
        r3.push(("XÓA".to_string(), 1.6, Kb::Back));
        rows.push(r3);
        rows.push(vec![ch('_'), ("HỦY".to_string(), 2.5, Kb::Cancel), ("XONG".to_string(), 3.5, Kb::Done)]);

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
                    if *code == Kb::Shift {
                        self.shift = !self.shift;
                    } else {
                        out = Some(*code);
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
