//! Cửa sổ vật chứa của server (rương, menu chọn chế độ chơi...): bố cục theo generic_54.png của Minecraft.
//! Phần thuần tính toán (không dính GL) để test được.

/// Số hàng ô vật chứa (1..=6) theo loại menu (registry menu_type) và số ô vật chứa.
pub fn rows_for(kind: i32, nc: usize) -> usize {
    match kind {
        0..=5 => kind as usize + 1, // generic_9x1 .. generic_9x6
        20 => 3,                    // shulker_box
        _ => nc.div_ceil(9).clamp(1, 6),
    }
}

/// Số ô vật chứa = tổng ô - 36 ô túi người chơi (27 túi + 9 thanh nhanh).
pub fn container_slots(total: usize) -> usize {
    total.saturating_sub(36)
}

pub struct Layout {
    /// khung toàn bộ [x, y, rộng, cao]
    pub panel: [f32; 4],
    /// nửa trên của ảnh nền: nguồn [0,0,176,17+18h] -> đích
    pub top_src: [f32; 4],
    pub top_dst: [f32; 4],
    /// nửa dưới (túi người chơi): nguồn [0,126,176,96] -> đích
    pub bot_src: [f32; 4],
    pub bot_dst: [f32; 4],
    /// vùng 16x16 vẽ vật phẩm của từng ô theo chỉ số ô của cửa sổ (0..nc, rồi 27 ô túi, rồi 9 thanh nhanh)
    pub slots: Vec<[f32; 4]>,
    /// vị trí chữ tiêu đề và nhãn "túi đồ"
    pub title_at: (f32, f32),
    pub label_at: (f32, f32),
    /// cỡ đơn vị GUI
    pub k: f32,
}

/// Hệ số phóng GUI nguyên (như Minecraft): lớn nhất còn vừa 320x240.
pub fn gui_scale(w: f32, h: f32) -> f32 {
    (w / 320.0).floor().min((h / 240.0).floor()).max(1.0)
}

pub fn layout(w: f32, h: f32, rows: usize, nc: usize) -> Layout {
    let k = gui_scale(w, h);
    let rows = rows.clamp(1, 6);
    let ph = (114 + rows * 18) as f32;
    let (px, py) = ((w - 176.0 * k) / 2.0, (h - ph * k) / 2.0);
    let at = |x: f32, y: f32| [px + x * k, py + y * k, 16.0 * k, 16.0 * k];
    let mut slots = Vec::with_capacity(nc + 36);
    for i in 0..nc {
        slots.push(at(8.0 + 18.0 * (i % 9) as f32, 18.0 + 18.0 * (i / 9) as f32));
    }
    let r = rows as f32;
    for j in 0..27 {
        slots.push(at(8.0 + 18.0 * (j % 9) as f32, 31.0 + 18.0 * r + 18.0 * (j / 9) as f32));
    }
    for j in 0..9 {
        slots.push(at(8.0 + 18.0 * j as f32, 89.0 + 18.0 * r));
    }
    let top_h = 17.0 + 18.0 * r;
    Layout {
        panel: [px, py, 176.0 * k, ph * k],
        top_src: [0.0, 0.0, 176.0, top_h],
        top_dst: [px, py, 176.0 * k, top_h * k],
        bot_src: [0.0, 126.0, 176.0, 96.0],
        bot_dst: [px, py + top_h * k, 176.0 * k, 96.0 * k],
        slots,
        title_at: (px + 8.0 * k, py + 6.0 * k),
        label_at: (px + 8.0 * k, py + (20.0 + 18.0 * r) * k),
        k,
    }
}

/// Ô nằm dưới điểm (x, y) (vùng chạm 18x18 quanh ô 16x16).
pub fn hit(l: &Layout, x: f32, y: f32) -> Option<usize> {
    l.slots.iter().position(|r| x >= r[0] - l.k && x < r[0] + r[2] + l.k && y >= r[1] - l.k && y < r[1] + r[3] + l.k)
}

/// Bỏ mã màu § (để vẽ chữ tối màu trên nền xám của khung).
pub fn strip_codes(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '§' {
            it.next();
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_by_kind() {
        assert_eq!(rows_for(0, 9), 1);
        assert_eq!(rows_for(2, 27), 3);
        assert_eq!(rows_for(5, 54), 6);
        assert_eq!(rows_for(20, 27), 3);
        assert_eq!(rows_for(16, 5), 1); // hopper
        assert_eq!(rows_for(8, 3), 1); // anvil
        assert_eq!(container_slots(63), 27);
        assert_eq!(container_slots(10), 0);
    }

    #[test]
    fn matches_vanilla_three_row_chest() {
        // rương 3 hàng của vanilla: ô vật chứa đầu (8,18); túi hàng 1 y=85; thanh nhanh y=143; cao 168
        let l = layout(1280.0, 720.0, 3, 27);
        let (px, py, k) = (l.panel[0], l.panel[1], l.k);
        assert_eq!(l.slots.len(), 27 + 36);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(near(l.slots[0][0], px + 8.0 * k) && near(l.slots[0][1], py + 18.0 * k));
        assert!(near(l.slots[27][1], py + 85.0 * k));
        assert!(near(l.slots[27 + 27][1], py + 143.0 * k));
        assert!(near(l.panel[3], 168.0 * k));
        // vanilla vẽ 17+18h hàng trên rồi 96 hàng dưới (167 điểm, khung 168 chỉ để căn giữa)
        assert!(near(l.top_dst[3] + l.bot_dst[3], 167.0 * k));
        assert!(near(l.bot_dst[1], l.top_dst[1] + l.top_dst[3]));
    }

    #[test]
    fn slots_inside_panel_and_not_overlapping() {
        for rows in 1..=6 {
            let l = layout(2400.0, 1080.0, rows, rows * 9);
            for (i, a) in l.slots.iter().enumerate() {
                assert!(a[0] >= l.panel[0] && a[0] + a[2] <= l.panel[0] + l.panel[2], "ô {i} ngoài khung ngang");
                assert!(a[1] >= l.panel[1] && a[1] + a[3] <= l.panel[1] + l.panel[3], "ô {i} ngoài khung dọc (hàng {rows})");
                for (j, b) in l.slots.iter().enumerate().skip(i + 1) {
                    let ov = a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3];
                    assert!(!ov, "ô {i} chồng ô {j}");
                }
            }
        }
    }

    #[test]
    fn hit_test_finds_slot_centres() {
        let l = layout(1920.0, 1080.0, 3, 27);
        for (i, r) in l.slots.iter().enumerate() {
            assert_eq!(hit(&l, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0), Some(i));
        }
        assert_eq!(hit(&l, 1.0, 1.0), None);
    }

    #[test]
    fn strip_color_codes() {
        assert_eq!(strip_codes("§aSkyblock §l§6Hot"), "Skyblock Hot");
        assert_eq!(strip_codes("Plain"), "Plain");
    }
}
