//! Thế giới khối: sinh địa hình, sửa khối và dựng mesh. Chỉ dùng thư viện chuẩn.

use std::collections::HashMap;

/// Kích thước chunk theo x/z và chiều cao.
pub const CS: i32 = 16;
pub const CH: i32 = 64;

pub const AIR: u8 = 0;
pub const GRASS: u8 = 1;
pub const DIRT: u8 = 2;
pub const STONE: u8 = 3;
pub const SAND: u8 = 4;
pub const PLANKS: u8 = 5;
pub const BRICK: u8 = 6;

pub const HOTBAR_SLOTS: usize = 6;
/// Các loại khối người chơi đặt được, theo thứ tự trên thanh vật phẩm.
pub const HOTBAR: [u8; HOTBAR_SLOTS] = [GRASS, DIRT, STONE, SAND, PLANKS, BRICK];

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub color: [f32; 3],
}

#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

// ---------- Noise ----------

fn hash2(x: i32, z: i32) -> f32 {
    let mut n = (x as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((z as u32).wrapping_mul(668_265_263));
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    n ^= n >> 16;
    (n & 0xFFFF) as f32 / 65535.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn value_noise(x: f32, z: f32) -> f32 {
    let (xf, zf) = (x.floor(), z.floor());
    let (fx, fz) = (smooth(x - xf), smooth(z - zf));
    let (xi, zi) = (xf as i32, zf as i32);
    let a = hash2(xi, zi);
    let b = hash2(xi + 1, zi);
    let c = hash2(xi, zi + 1);
    let d = hash2(xi + 1, zi + 1);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    top + (bottom - top) * fz
}

/// Độ cao mặt đất (chỉ số y của khối trên cùng) tại cột (x, z).
pub fn height_at(x: i32, z: i32) -> i32 {
    let (fx, fz) = (x as f32, z as f32);
    let n = 0.6 * value_noise(fx * 0.02, fz * 0.02)
        + 0.3 * value_noise(fx * 0.06, fz * 0.06)
        + 0.1 * value_noise(fx * 0.18, fz * 0.18);
    (12.0 + n * 28.0) as i32
}

fn column_block(h: i32, y: i32) -> u8 {
    if y > h {
        AIR
    } else if y == h {
        if h < 18 {
            SAND
        } else {
            GRASS
        }
    } else if y >= h - 3 {
        DIRT
    } else {
        STONE
    }
}

fn gen_block(x: i32, y: i32, z: i32) -> u8 {
    column_block(height_at(x, z), y)
}

fn idx(lx: i32, y: i32, lz: i32) -> usize {
    ((y * CS + lz) * CS + lx) as usize
}

fn gen_chunk(cx: i32, cz: i32) -> Vec<u8> {
    let mut data = vec![AIR; (CS * CS * CH) as usize];
    for lz in 0..CS {
        for lx in 0..CS {
            let h = height_at(cx * CS + lx, cz * CS + lz);
            for y in 0..CH {
                data[idx(lx, y, lz)] = column_block(h, y);
            }
        }
    }
    data
}

// ---------- Mặt khối ----------

struct Face {
    n: [i32; 3],
    shade: f32,
    v: [[f32; 3]; 4],
}

// Bốn đỉnh của mỗi mặt xếp ngược chiều kim đồng hồ khi nhìn từ bên ngoài.
const FACES: [Face; 6] = [
    Face { n: [0, 1, 0], shade: 1.0, v: [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]] },
    Face { n: [0, -1, 0], shade: 0.5, v: [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]] },
    Face { n: [1, 0, 0], shade: 0.8, v: [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]] },
    Face { n: [-1, 0, 0], shade: 0.8, v: [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]] },
    Face { n: [0, 0, 1], shade: 0.65, v: [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]] },
    Face { n: [0, 0, -1], shade: 0.65, v: [[0., 0., 0.], [0., 1., 0.], [1., 1., 0.], [1., 0., 0.]] },
];

/// Màu khối (dạng sRGB, 0..1).
pub fn block_color(b: u8) -> [f32; 3] {
    match b {
        GRASS => [0.36, 0.66, 0.25],
        DIRT => [0.52, 0.36, 0.22],
        STONE => [0.50, 0.50, 0.53],
        SAND => [0.86, 0.80, 0.56],
        PLANKS => [0.72, 0.56, 0.33],
        BRICK => [0.62, 0.28, 0.24],
        _ => [1.0, 0.0, 1.0],
    }
}

/// Đổi gần đúng sRGB -> tuyến tính (bề mặt hiển thị là sRGB).
fn lin(c: f32) -> f32 {
    c * c
}

/// Thêm một hộp [min, max] vào mesh, tô một màu (đã ở dạng tuyến tính), không đổ bóng.
fn push_box(m: &mut MeshData, min: [f32; 3], max: [f32; 3], color: [f32; 3]) {
    for f in &FACES {
        let start = m.vertices.len() as u32;
        for c in &f.v {
            m.vertices.push(Vertex {
                pos: [
                    min[0] + c[0] * (max[0] - min[0]),
                    min[1] + c[1] * (max[1] - min[1]),
                    min[2] + c[2] * (max[2] - min[2]),
                ],
                color,
            });
        }
        m.indices
            .extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
    }
}

pub const OUTLINE_MAX_VERTICES: usize = 288;
pub const OUTLINE_MAX_INDICES: usize = 432;

/// Khung viền 12 cạnh quanh khối (x, y, z), để người chơi biết đang nhắm khối nào.
pub fn outline_mesh(x: i32, y: i32, z: i32) -> MeshData {
    const T: f32 = 0.018; // độ dày cạnh
    const E: f32 = 0.004; // phình ra một chút để không bị z-fighting
    let (x0, y0, z0) = (x as f32 - E, y as f32 - E, z as f32 - E);
    let (x1, y1, z1) = (x as f32 + 1.0 + E, y as f32 + 1.0 + E, z as f32 + 1.0 + E);
    let color = [0.01, 0.01, 0.01];
    let mut m = MeshData::default();
    for &a in &[y0, y1 - T] {
        for &b in &[z0, z1 - T] {
            push_box(&mut m, [x0, a, b], [x1, a + T, b + T], color);
        }
    }
    for &a in &[x0, x1 - T] {
        for &b in &[z0, z1 - T] {
            push_box(&mut m, [a, y0, b], [a + T, y1, b + T], color);
        }
    }
    for &a in &[x0, x1 - T] {
        for &b in &[y0, y1 - T] {
            push_box(&mut m, [a, b, z0], [a + T, b + T, z1], color);
        }
    }
    m
}

// ---------- Thế giới ----------

#[derive(Default)]
pub struct World {
    chunks: HashMap<(i32, i32), Vec<u8>>,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sinh dữ liệu chunk nếu chưa có. Phải gọi trước `mesh_chunk`.
    pub fn ensure_chunk(&mut self, cx: i32, cz: i32) {
        self.chunks.entry((cx, cz)).or_insert_with(|| gen_chunk(cx, cz));
    }

    pub fn has_chunk(&self, cx: i32, cz: i32) -> bool {
        self.chunks.contains_key(&(cx, cz))
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Chunk chưa sinh thì đọc thẳng từ bộ sinh địa hình (kết quả giống hệt).
    pub fn get(&self, x: i32, y: i32, z: i32) -> u8 {
        if y < 0 {
            return STONE;
        }
        if y >= CH {
            return AIR;
        }
        match self.chunks.get(&(x.div_euclid(CS), z.div_euclid(CS))) {
            Some(c) => c[idx(x.rem_euclid(CS), y, z.rem_euclid(CS))],
            None => gen_block(x, y, z),
        }
    }

    pub fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        self.get(x, y, z) != AIR
    }

    /// Đặt khối. Trả về các chunk cần dựng lại mesh (chunk chứa khối, cộng chunk kề nếu khối nằm ở rìa).
    pub fn set(&mut self, x: i32, y: i32, z: i32, b: u8) -> Vec<(i32, i32)> {
        if !(0..CH).contains(&y) {
            return Vec::new();
        }
        let (cx, cz) = (x.div_euclid(CS), z.div_euclid(CS));
        let (lx, lz) = (x.rem_euclid(CS), z.rem_euclid(CS));
        self.ensure_chunk(cx, cz);
        let Some(chunk) = self.chunks.get_mut(&(cx, cz)) else {
            return Vec::new();
        };
        let i = idx(lx, y, lz);
        if chunk[i] == b {
            return Vec::new();
        }
        chunk[i] = b;

        let mut dirty = vec![(cx, cz)];
        if lx == 0 {
            dirty.push((cx - 1, cz));
        }
        if lx == CS - 1 {
            dirty.push((cx + 1, cz));
        }
        if lz == 0 {
            dirty.push((cx, cz - 1));
        }
        if lz == CS - 1 {
            dirty.push((cx, cz + 1));
        }
        dirty
    }

    /// Dựng mesh cho chunk (cần `ensure_chunk` trước, chưa có thì trả về mesh rỗng).
    pub fn mesh_chunk(&self, cx: i32, cz: i32) -> MeshData {
        let mut m = MeshData::default();
        let Some(data) = self.chunks.get(&(cx, cz)) else {
            return m;
        };

        // Đọc trong chunk thì chỉ số trực tiếp, ra ngoài rìa mới tra bảng băm.
        let block_at = |lx: i32, y: i32, lz: i32| -> u8 {
            if y < 0 {
                STONE
            } else if y >= CH {
                AIR
            } else if (0..CS).contains(&lx) && (0..CS).contains(&lz) {
                data[idx(lx, y, lz)]
            } else {
                self.get(cx * CS + lx, y, cz * CS + lz)
            }
        };

        for y in 0..CH {
            for lz in 0..CS {
                for lx in 0..CS {
                    let b = data[idx(lx, y, lz)];
                    if b == AIR {
                        continue;
                    }
                    let (x, z) = (cx * CS + lx, cz * CS + lz);
                    let base = block_color(b);
                    // Biến thiên nhẹ theo vị trí cho đỡ trơn.
                    let var = 0.92 + 0.08 * hash2(x + y * 31, z + y * 17);
                    for f in &FACES {
                        if block_at(lx + f.n[0], y + f.n[1], lz + f.n[2]) != AIR {
                            continue;
                        }
                        let k = f.shade * var;
                        let color = [lin(base[0] * k), lin(base[1] * k), lin(base[2] * k)];
                        let start = m.vertices.len() as u32;
                        for c in &f.v {
                            m.vertices.push(Vertex {
                                pos: [x as f32 + c[0], y as f32 + c[1], z as f32 + c[2]],
                                color,
                            });
                        }
                        m.indices.extend_from_slice(&[
                            start,
                            start + 1,
                            start + 2,
                            start,
                            start + 2,
                            start + 3,
                        ]);
                    }
                }
            }
        }
        m
    }

    /// Chỉ dùng trong test: sinh sẵn lưới chunk từ -radius đến radius-1.
    #[cfg(test)]
    pub fn generate_square(&mut self, radius: i32) {
        for cx in -radius..radius {
            for cz in -radius..radius {
                self.ensure_chunk(cx, cz);
            }
        }
    }

    #[cfg(test)]
    pub fn chunk_coords(&self) -> Vec<(i32, i32)> {
        let mut v: Vec<_> = self.chunks.keys().copied().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn face_winding_points_outward() {
        for f in &FACES {
            let n = [f.n[0] as f32, f.n[1] as f32, f.n[2] as f32];
            for tri in [[0, 1, 2], [0, 2, 3]] {
                let (a, b, c) = (f.v[tri[0]], f.v[tri[1]], f.v[tri[2]]);
                let normal = cross(sub(b, a), sub(c, a));
                assert!(dot(normal, n) > 0.0, "mặt {:?} bị lật", f.n);
            }
        }
    }

    #[test]
    fn heights_in_range() {
        for x in (-200..200).step_by(7) {
            for z in (-200..200).step_by(11) {
                let h = height_at(x, z);
                assert!((0..CH - 1).contains(&h), "h={h} ngoài khoảng");
            }
        }
    }

    #[test]
    fn mesh_is_sane_and_chunks_agree() {
        let mut w = World::new();
        w.generate_square(2);
        let mut total_quads = 0;
        for (cx, cz) in w.chunk_coords() {
            let m = w.mesh_chunk(cx, cz);
            assert_eq!(m.indices.len() % 6, 0);
            assert_eq!(m.vertices.len() / 4 * 6, m.indices.len());
            assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
            total_quads += m.indices.len() / 6;
        }
        assert!(total_quads > 1000);
        // Khối đọc qua chunk phải trùng với bộ sinh địa hình.
        for (x, z) in [(-33, -33), (-1, 0), (0, -1), (31, 31), (17, -9)] {
            for y in [0, 10, 25, 50] {
                assert_eq!(w.get(x, y, z), gen_block(x, y, z), "({x},{y},{z})");
            }
        }
        // Ngoài lưới vẫn có dữ liệu liền mạch.
        assert_eq!(w.get(500, 0, 500), gen_block(500, 0, 500));
    }

    #[test]
    fn mesh_without_data_is_empty() {
        let w = World::new();
        assert!(w.mesh_chunk(7, 7).indices.is_empty());
    }

    #[test]
    fn top_faces_only_on_surface() {
        let mut w = World::new();
        w.generate_square(1);
        let h = height_at(3, 3);
        assert_ne!(w.get(3, h, 3), AIR);
        assert_eq!(w.get(3, h + 1, 3), AIR);
    }

    #[test]
    fn set_get_and_dirty_chunks() {
        let mut w = World::new();
        assert_eq!(w.set(5, 30, 5, STONE), vec![(0, 0)]);
        assert_eq!(w.get(5, 30, 5), STONE);
        // Đặt lại cùng khối thì không có gì đổi.
        assert!(w.set(5, 30, 5, STONE).is_empty());
        // Khối ở rìa chunk kéo theo chunk kề.
        let d = w.set(16, 30, 5, STONE);
        assert!(d.contains(&(1, 0)) && d.contains(&(0, 0)), "{d:?}");
        let d = w.set(-1, 30, -1, STONE);
        for c in [(-1, -1), (0, -1), (-1, 0)] {
            assert!(d.contains(&c), "thiếu {c:?} trong {d:?}");
        }
        // Ngoài chiều cao thì bỏ qua.
        assert!(w.set(0, -1, 0, STONE).is_empty());
        assert!(w.set(0, CH, 0, STONE).is_empty());
        assert_eq!(w.get(0, -1, 0), STONE);
    }

    #[test]
    fn editing_changes_mesh_and_round_trips() {
        let mut w = World::new();
        w.generate_square(1);
        let h = height_at(3, 3);
        let before = w.mesh_chunk(0, 0).indices.len();
        let dirty = w.set(3, h + 5, 3, STONE);
        assert!(dirty.contains(&(0, 0)));
        // Một khối lơ lửng thêm đúng 6 mặt.
        assert_eq!(w.mesh_chunk(0, 0).indices.len(), before + 36);
        w.set(3, h + 5, 3, AIR);
        assert_eq!(w.mesh_chunk(0, 0).indices.len(), before);
    }

    #[test]
    fn neighbor_edit_updates_border_faces() {
        let mut w = World::new();
        w.generate_square(2);
        // Đào khối sát rìa chunk (0,0) ở độ cao mà khối kề bên chunk (-1,0) chắc chắn đặc:
        // chunk (-1,0) phải lộ thêm đúng một mặt hướng +x.
        let y = height_at(0, 4).min(height_at(-1, 4));
        assert!(w.is_solid(0, y, 4) && w.is_solid(-1, y, 4));
        let before = w.mesh_chunk(-1, 0).indices.len();
        let dirty = w.set(0, y, 4, AIR);
        assert!(dirty.contains(&(-1, 0)));
        assert_eq!(w.mesh_chunk(-1, 0).indices.len(), before + 6);
    }

    #[test]
    fn outline_mesh_is_sane() {
        let m = outline_mesh(10, 20, -5);
        assert_eq!(m.vertices.len(), OUTLINE_MAX_VERTICES);
        assert_eq!(m.indices.len(), OUTLINE_MAX_INDICES);
        assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
        for v in &m.vertices {
            assert!(v.pos[0] >= 9.99 && v.pos[0] <= 11.01);
            assert!(v.pos[1] >= 19.99 && v.pos[1] <= 21.01);
            assert!(v.pos[2] >= -5.01 && v.pos[2] <= -3.99);
        }
    }

    #[test]
    fn hotbar_blocks_are_placeable_types() {
        for b in HOTBAR {
            assert_ne!(b, AIR);
            assert_ne!(block_color(b), [1.0, 0.0, 1.0], "khối {b} thiếu màu");
        }
    }
}
