//! Túi đồ người chơi: 9 ô thanh nhanh (0..8), 27 ô túi (9..35), 4 ô giáp, tay phụ, lưới chế tạo 2x2 và ô kết quả.
//! Con trỏ cầm vật phẩm (kéo/thả giữa các ô).
use crate::craft::{self, Recipe};

pub const MAX_STACK: u8 = 64;
/// 0..8 thanh nhanh, 9..35 túi, 36..39 giáp (mũ, áo, quần, giày), 40 tay phụ, 41..44 lưới chế tạo, 45 kết quả
pub const SLOTS: usize = 46;
pub const ARMOR0: usize = 36;
pub const OFFHAND: usize = 40;
pub const CRAFT0: usize = 41;
pub const RESULT: usize = 45;
/// số ô có thể cất đồ khi đóng túi (thanh nhanh + túi)
const STORE: usize = 36;

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
    /// theo vật phẩm: -1 không mặc được, 0 mũ, 1 áo, 2 quần, 3 giày
    wear: Vec<i8>,
    recipes: Vec<Recipe>,
}

impl Inv {
    /// Túi thử nghiệm: 36 vật phẩm đầu mỗi loại một ô 64 cái (thanh nhanh trước); các vật còn lại chỉ có qua chế tạo.
    pub fn new(n_items: usize, wear: Vec<i8>, recipes: Vec<Recipe>) -> Inv {
        let _ = n_items;
        let slots = [None; SLOTS];
        Inv { slots, sel: 0, cursor: None, wear, recipes }
    }

    fn can_wear(&self, item: u16, slot: usize) -> bool {
        self.wear.get(item as usize).copied() == Some((slot - ARMOR0) as i8)
    }

    fn update_result(&mut self) {
        let g = [self.slots[CRAFT0].map(|s| s.item), self.slots[CRAFT0 + 1].map(|s| s.item), self.slots[CRAFT0 + 2].map(|s| s.item), self.slots[CRAFT0 + 3].map(|s| s.item)];
        self.slots[RESULT] = craft::find(&g, &self.recipes).map(|(item, n)| Stack { item, n });
    }

    /// Chạm ô `i`: lấy lên / thả xuống / gộp / đổi chỗ với thứ đang cầm. Ô kết quả chỉ để lấy; ô giáp chỉ nhận đúng loại.
    pub fn click(&mut self, i: usize) {
        if i >= SLOTS {
            return;
        }
        if i == RESULT {
            let Some(r) = self.slots[RESULT] else { return };
            match self.cursor {
                None => self.cursor = Some(r),
                Some(c) if c.item == r.item && c.n as u16 + r.n as u16 <= MAX_STACK as u16 => self.cursor = Some(Stack { item: c.item, n: c.n + r.n }),
                _ => return,
            }
            for k in CRAFT0..CRAFT0 + 4 {
                if let Some(s) = self.slots[k].as_mut() {
                    s.n -= 1;
                    if s.n == 0 {
                        self.slots[k] = None;
                    }
                }
            }
            self.update_result();
            return;
        }
        let armor = (ARMOR0..ARMOR0 + 4).contains(&i);
        if armor {
            // mỗi ô giáp chứa đúng 1 món, đúng loại
            match (self.cursor, self.slots[i]) {
                (Some(c), None) if self.can_wear(c.item, i) => {
                    self.slots[i] = Some(Stack { item: c.item, n: 1 });
                    self.cursor = if c.n > 1 { Some(Stack { item: c.item, n: c.n - 1 }) } else { None };
                }
                (None, Some(s)) => {
                    self.cursor = Some(s);
                    self.slots[i] = None;
                }
                (Some(c), Some(s)) if c.n == 1 && self.can_wear(c.item, i) => {
                    self.slots[i] = Some(c);
                    self.cursor = Some(s);
                }
                _ => {}
            }
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
        if (CRAFT0..CRAFT0 + 4).contains(&i) {
            self.update_result();
        }
    }

    fn add(&mut self, mut c: Stack) {
        for s in self.slots[..STORE].iter_mut().flatten() {
            if s.item == c.item && s.n < MAX_STACK {
                let put = (MAX_STACK - s.n).min(c.n);
                s.n += put;
                c.n -= put;
                if c.n == 0 {
                    return;
                }
            }
        }
        for s in self.slots[..STORE].iter_mut() {
            if s.is_none() {
                *s = Some(c);
                return;
            }
        }
    }

    /// Cất thứ đang cầm và đồ trong lưới chế tạo vào túi (khi đóng túi). Hết chỗ thì bỏ.
    pub fn stash(&mut self) {
        if let Some(c) = self.cursor.take() {
            self.add(c);
        }
        for k in CRAFT0..CRAFT0 + 4 {
            if let Some(s) = self.slots[k].take() {
                self.add(s);
            }
        }
        self.update_result();
    }

    pub fn held(&self) -> Option<u16> {
        self.slots[self.sel].map(|s| s.item)
    }
}
