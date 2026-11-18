//! Túi đồ người chơi: 9 ô thanh nhanh (0..8) + 27 ô túi (9..35), con trỏ cầm vật phẩm (kéo/thả giữa các ô).

pub const MAX_STACK: u8 = 64;
pub const SLOTS: usize = 36;

#[derive(Clone, Copy, PartialEq)]
pub struct Stack {
    /// chỉ số trong danh sách vật phẩm (items::Items::list)
    pub item: u16,
    pub n: u8,
}

pub struct Inv {
    pub slots: [Option<Stack>; SLOTS],
    pub sel: usize,
    pub cursor: Option<Stack>,
}

impl Inv {
    /// Túi thử nghiệm: mỗi vật phẩm một ô 64 cái, theo thứ tự (thanh nhanh trước).
    pub fn new(n_items: usize) -> Inv {
        let mut slots = [None; SLOTS];
        for (i, s) in slots.iter_mut().enumerate().take(n_items) {
            *s = Some(Stack { item: i as u16, n: MAX_STACK });
        }
        Inv { slots, sel: 0, cursor: None }
    }

    /// Chạm ô `i`: lấy lên / thả xuống / gộp / đổi chỗ với thứ đang cầm.
    pub fn click(&mut self, i: usize) {
        if i >= SLOTS {
            return;
        }
        match (self.cursor, self.slots[i]) {
            (None, s) => {
                self.cursor = s;
                self.slots[i] = None;
            }
            (Some(c), None) => {
                self.slots[i] = Some(c);
                self.cursor = None;
            }
            (Some(c), Some(s)) if c.item == s.item => {
                let total = c.n as u16 + s.n as u16;
                let put = total.min(MAX_STACK as u16);
                self.slots[i] = Some(Stack { item: s.item, n: put as u8 });
                let rest = total - put;
                self.cursor = if rest > 0 { Some(Stack { item: c.item, n: rest as u8 }) } else { None };
            }
            (Some(c), Some(s)) => {
                self.slots[i] = Some(c);
                self.cursor = Some(s);
            }
        }
    }

    /// Cất thứ đang cầm vào ô trống / ô cùng loại (khi đóng túi). Hết chỗ thì bỏ.
    pub fn stash(&mut self) {
        let Some(mut c) = self.cursor.take() else { return };
        for s in self.slots.iter_mut().flatten() {
            if s.item == c.item && s.n < MAX_STACK {
                let put = (MAX_STACK - s.n).min(c.n);
                s.n += put;
                c.n -= put;
                if c.n == 0 {
                    return;
                }
            }
        }
        for s in self.slots.iter_mut() {
            if s.is_none() {
                *s = Some(c);
                return;
            }
        }
    }

    pub fn held(&self) -> Option<u16> {
        self.slots[self.sel].map(|s| s.item)
    }
}
