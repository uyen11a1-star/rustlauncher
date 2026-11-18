//! Công thức chế tạo 2x2 đọc từ data/*/recipes của client.jar: khớp lưới (có lật gương) và công thức không thứ tự.

pub struct Recipe {
    /// có hình dạng (cells là lưới w x h, rỗng = ô trống) hay không thứ tự (cells là danh sách nguyên liệu)
    pub shaped: bool,
    pub w: usize,
    pub h: usize,
    /// mỗi ô: các vật phẩm chấp nhận được (chỉ số trong danh sách vật phẩm)
    pub cells: Vec<Vec<u16>>,
    pub out: (u16, u8),
}

fn shaped_match(r: &Recipe, grid: &[Option<u16>; 4], r0: usize, c0: usize, mirror: bool) -> bool {
    for y in 0..r.h {
        for x in 0..r.w {
            let rx = if mirror { r.w - 1 - x } else { x };
            let want = &r.cells[y * r.w + rx];
            let have = grid[(r0 + y) * 2 + c0 + x];
            match have {
                None => {
                    if !want.is_empty() {
                        return false;
                    }
                }
                Some(it) => {
                    if !want.contains(&it) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn assign(ings: &[Vec<u16>], items: &[u16], used: &mut Vec<bool>, k: usize) -> bool {
    if k == ings.len() {
        return true;
    }
    for (j, it) in items.iter().enumerate() {
        if !used[j] && ings[k].contains(it) {
            used[j] = true;
            if assign(ings, items, used, k + 1) {
                return true;
            }
            used[j] = false;
        }
    }
    false
}

/// Kết quả chế tạo của lưới 2x2 (hàng trước: ô 0,1; hàng sau: ô 2,3), nếu có công thức khớp.
pub fn find(grid: &[Option<u16>; 4], rs: &[Recipe]) -> Option<(u16, u8)> {
    let filled: Vec<usize> = (0..4).filter(|&i| grid[i].is_some()).collect();
    if filled.is_empty() {
        return None;
    }
    let (rmin, rmax) = (filled.iter().map(|i| i / 2).min().unwrap(), filled.iter().map(|i| i / 2).max().unwrap());
    let (cmin, cmax) = (filled.iter().map(|i| i % 2).min().unwrap(), filled.iter().map(|i| i % 2).max().unwrap());
    let (gh, gw) = (rmax - rmin + 1, cmax - cmin + 1);
    let items: Vec<u16> = filled.iter().map(|&i| grid[i].unwrap()).collect();
    for r in rs {
        if r.shaped {
            if r.w == gw && r.h == gh && (shaped_match(r, grid, rmin, cmin, false) || shaped_match(r, grid, rmin, cmin, true)) {
                return Some(r.out);
            }
        } else if r.cells.len() == items.len() {
            let mut used = vec![false; items.len()];
            if assign(&r.cells, &items, &mut used, 0) {
                return Some(r.out);
            }
        }
    }
    None
}
