//! Gói tin cửa sổ vật chứa (rương, menu server) của giao thức 1.21.9/1.21.10: phân tích và dựng gói.
use crate::mcgui::net::{put_varint, rd_varint};

fn rd_i16(b: &[u8], i: &mut usize) -> Option<i16> {
    let s = b.get(*i..*i + 2)?;
    *i += 2;
    Some(i16::from_be_bytes([s[0], s[1]]))
}

/// Ô vật phẩm của cửa sổ vật chứa: (id mạng, số lượng, tên tuỳ chỉnh có mã §).
pub type NSlot = Option<(u32, u8, Option<String>)>;

/// Cửa sổ vật chứa server đang mở (rương, menu chọn chế độ chơi...).
#[derive(Clone)]
pub struct NetWin {
    pub id: i32,
    /// loại menu theo registry menu_type (0..5 = rương 9x1..9x6)
    pub kind: i32,
    pub title: String,
    pub state_id: i32,
    /// toàn bộ ô theo thứ tự của gói: ô vật chứa, rồi 27 ô túi, rồi 9 thanh nhanh
    pub slots: Vec<NSlot>,
}

/// Gói open_window (0x39): id, loại, tiêu đề.
pub fn parse_open_window(b: &[u8]) -> Option<NetWin> {
    let mut i = 0;
    let id = rd_varint(b, &mut i)?;
    let kind = rd_varint(b, &mut i)?;
    let title = crate::chat::read_nbt(b, &mut i).map(|t| crate::chat::to_legacy(&t)).unwrap_or_default();
    Some(NetWin { id, kind, title, state_id: 0, slots: Vec::new() })
}

/// Gói window_items (0x12): (id cửa sổ, state id, các ô).
pub fn parse_window_items(b: &[u8]) -> Option<(i32, i32, Vec<NSlot>)> {
    let mut i = 0;
    let id = rd_varint(b, &mut i)?;
    let state = rd_varint(b, &mut i)?;
    let n = rd_varint(b, &mut i)?;
    if !(0..=256).contains(&n) {
        return None;
    }
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        v.push(crate::slot::read_slot_named(b, &mut i)?);
    }
    Some((id, state, v))
}

/// Gói set_slot (0x14): (id cửa sổ, state id, chỉ số ô, vật phẩm).
pub fn parse_set_slot(b: &[u8]) -> Option<(i32, i32, i16, NSlot)> {
    let mut i = 0;
    let id = rd_varint(b, &mut i)?;
    let state = rd_varint(b, &mut i)?;
    let slot = rd_i16(b, &mut i)?;
    Some((id, state, slot, crate::slot::read_slot_named(b, &mut i)?))
}

/// Áp một gói cửa sổ của server (0x39 mở, 0x11 đóng, 0x12 toàn bộ ô, 0x14 một ô) vào trạng thái `win`.
/// Gói của cửa sổ khác cửa sổ đang mở (hoặc của cửa sổ 0 = túi người chơi) bị bỏ qua.
pub fn apply(win: &mut Option<NetWin>, dirty: &mut bool, pid: i32, b: &[u8]) {
    match pid {
        0x39 => {
            crate::netplay::trace(format!("WIN mở: {}", parse_open_window(b).map_or("LỖI PHÂN TÍCH".to_string(), |w| format!("id={} loại={} tiêu đề={}", w.id, w.kind, w.title))));
            if let Some(w) = parse_open_window(b) {
                *win = Some(w);
                *dirty = true;
            }
        }
        0x11 => {
            let mut i = 0;
            let wid = rd_varint(b, &mut i).unwrap_or(-1);
            if win.as_ref().map_or(false, |w| w.id == wid) {
                *win = None;
                *dirty = true;
            }
        }
        0x12 => {
            let r = parse_window_items(b);
            crate::netplay::trace(format!("WIN items: {}", r.as_ref().map_or("LỖI PHÂN TÍCH (slot lạ?)".to_string(), |(w, st, v)| format!("id={w} state={st} ô={} cửa sổ đang mở={:?}", v.len(), win.as_ref().map(|x| x.id)))));
            if let Some((wid, state, slots)) = r {
                if let Some(w) = win.as_mut().filter(|w| w.id == wid) {
                    w.state_id = state;
                    w.slots = slots;
                    *dirty = true;
                }
            }
        }
        0x14 => {
            if let Some((wid, state, slot, item)) = parse_set_slot(b) {
                if let Some(w) = win.as_mut().filter(|w| w.id == wid && (0..256).contains(&slot)) {
                    let s = slot as usize;
                    if w.slots.len() <= s {
                        w.slots.resize(s + 1, None);
                    }
                    w.slots[s] = item;
                    w.state_id = state;
                    *dirty = true;
                }
            }
        }
        _ => {}
    }
}

/// Thân gói window_click (0x11) không thay đổi ô nào và tay trống (đủ cho bấm chọn trong menu server).
pub fn click_packet(win: i32, state: i32, slot: i16, button: i8, mode: i32) -> Vec<u8> {
    let mut b = Vec::new();
    put_varint(&mut b, win as u32);
    put_varint(&mut b, state as u32);
    b.extend_from_slice(&slot.to_be_bytes());
    b.push(button as u8);
    put_varint(&mut b, mode as u32);
    put_varint(&mut b, 0); // changedSlots: rỗng
    b.push(0); // cursorItem: không có
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vi(out: &mut Vec<u8>, v: u32) {
        put_varint(out, v);
    }

    /// NBT mạng không tên gốc kiểu chuỗi: thành phần văn bản dạng chuỗi.
    fn nbt_str(s: &str) -> Vec<u8> {
        let mut v = vec![8u8];
        v.extend_from_slice(&(s.len() as u16).to_be_bytes());
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// Slot có số lượng n, vật phẩm id, và (tuỳ chọn) component custom_name (loại 5).
    fn slot(id: u32, n: u32, name: Option<&str>) -> Vec<u8> {
        let mut b = Vec::new();
        vi(&mut b, n);
        if n == 0 {
            return b;
        }
        vi(&mut b, id);
        vi(&mut b, name.is_some() as u32);
        vi(&mut b, 0);
        if let Some(t) = name {
            vi(&mut b, 5);
            b.extend(nbt_str(t));
        }
        b
    }

    #[test]
    fn open_window_chest_with_title() {
        let mut b = Vec::new();
        vi(&mut b, 7); // id cửa sổ
        vi(&mut b, 2); // generic_9x3
        b.extend(nbt_str("Chon che do"));
        let w = parse_open_window(&b).unwrap();
        assert_eq!((w.id, w.kind), (7, 2));
        assert_eq!(w.title, "Chon che do");
        assert!(w.slots.is_empty());
    }

    #[test]
    fn window_items_with_named_compass() {
        let mut b = Vec::new();
        vi(&mut b, 7);
        vi(&mut b, 42); // state id
        vi(&mut b, 3);
        b.extend(slot(0, 0, None)); // ô trống
        b.extend(slot(1111, 1, Some("§aSkyblock")));
        b.extend(slot(25, 64, None));
        b.extend(slot(0, 0, None)); // carried item
        let (id, st, v) = parse_window_items(&b).unwrap();
        assert_eq!((id, st, v.len()), (7, 42, 3));
        assert_eq!(v[0], None);
        assert_eq!(v[1], Some((1111, 1, Some("§aSkyblock".to_string()))));
        assert_eq!(v[2], Some((25, 64, None)));
    }

    #[test]
    fn set_slot_parses_item() {
        let mut b = Vec::new();
        vi(&mut b, 7);
        vi(&mut b, 43);
        b.extend_from_slice(&13i16.to_be_bytes());
        b.extend(slot(5, 2, Some("Hello")));
        let (id, st, slot_i, item) = parse_set_slot(&b).unwrap();
        assert_eq!((id, st, slot_i), (7, 43, 13));
        assert_eq!(item, Some((5, 2, Some("Hello".to_string()))));
    }

    #[test]
    fn full_lifecycle_open_fill_update_close() {
        let (mut win, mut dirty) = (None, false);
        let mut open = Vec::new();
        vi(&mut open, 3);
        vi(&mut open, 2);
        open.extend(nbt_str("Menu"));
        apply(&mut win, &mut dirty, 0x39, &open);
        assert!(dirty && win.as_ref().map(|w| w.id) == Some(3));
        dirty = false;

        let mut items = Vec::new();
        vi(&mut items, 3);
        vi(&mut items, 10);
        vi(&mut items, 2);
        items.extend(slot(1111, 1, Some("§aSurvival")));
        items.extend(slot(0, 0, None));
        items.extend(slot(0, 0, None)); // carried
        apply(&mut win, &mut dirty, 0x12, &items);
        assert!(dirty);
        let w = win.as_ref().unwrap();
        assert_eq!((w.state_id, w.slots.len()), (10, 2));
        assert_eq!(w.slots[0].as_ref().unwrap().2.as_deref(), Some("§aSurvival"));
        dirty = false;

        // set_slot vào ô 5 (vượt độ dài hiện tại) làm mảng nở ra
        let mut one = Vec::new();
        vi(&mut one, 3);
        vi(&mut one, 11);
        one.extend_from_slice(&5i16.to_be_bytes());
        one.extend(slot(7, 3, None));
        apply(&mut win, &mut dirty, 0x14, &one);
        let w = win.as_ref().unwrap();
        assert!(dirty && w.slots.len() == 6 && w.state_id == 11);
        assert_eq!(w.slots[5], Some((7, 3, None)));
        dirty = false;

        // gói của cửa sổ khác / túi người chơi (id 0) không đụng vào cửa sổ này
        let mut other = Vec::new();
        vi(&mut other, 0);
        vi(&mut other, 1);
        other.extend_from_slice(&5i16.to_be_bytes());
        other.extend(slot(9, 9, None));
        apply(&mut win, &mut dirty, 0x14, &other);
        let mut close_other = Vec::new();
        vi(&mut close_other, 99);
        apply(&mut win, &mut dirty, 0x11, &close_other);
        assert!(!dirty && win.is_some());

        // server đóng đúng cửa sổ
        let mut close = Vec::new();
        vi(&mut close, 3);
        apply(&mut win, &mut dirty, 0x11, &close);
        assert!(dirty && win.is_none());
    }

    #[test]
    fn truncated_or_hostile_packets_return_none() {
        assert!(parse_window_items(&[1, 1]).is_none());
        let mut b = Vec::new();
        vi(&mut b, 1);
        vi(&mut b, 1);
        vi(&mut b, 100000); // số ô vô lý
        assert!(parse_window_items(&b).is_none());
        assert!(parse_open_window(&[]).is_none());
        assert!(parse_set_slot(&[1, 2, 3]).is_none());
    }

    #[test]
    fn click_packet_layout_matches_protocol() {
        // windowId, stateId, slot(i16), button(i8), mode, changedSlots=0, cursor=none
        let p = click_packet(7, 42, 13, 0, 0);
        assert_eq!(p, vec![7, 42, 0, 13, 0, 0, 0, 0]);
        let p = click_packet(1, 300, -999, 1, 4);
        // 300 = varint 0xAC 0x02; -999 = 0xFC19
        assert_eq!(p, vec![1, 0xAC, 0x02, 0xFC, 0x19, 1, 4, 0, 0]);
    }
}
