//! Thế giới khối: sinh địa hình và dựng mesh. Chỉ dùng thư viện chuẩn.

use std::collections::HashMap;

/// Kích thước chunk theo x/z và chiều cao.
pub const CS: i32 = 16;
pub const CH: i32 = 64;

pub const AIR: u8 = 0;
pub const GRASS: u8 = 1;
pub const DIRT: u8 = 2;
pub const STONE: u8 = 3;
pub const SAND: u8 = 4;

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

fn block_color(b: u8) -> [f32; 3] {
    match b {
        GRASS => [0.36, 0.66, 0.25],
        DIRT => [0.52, 0.36, 0.22],
        STONE => [0.50, 0.50, 0.53],
        SAND => [0.86, 0.80, 0.56],
        _ => [1.0, 0.0, 1.0],
    }
}

/// Đổi gần đúng sRGB -> tuyến tính (bề mặt hiển thị là sRGB).
fn lin(c: f32) -> f32 {
    c * c
}

// ---------- Thế giới ----------

pub struct World {
    chunks: HashMap<(i32, i32), Vec<u8>>,
}

impl World {
    /// Sinh lưới chunk từ -radius đến radius-1 theo cả hai trục.
    pub fn new(radius: i32) -> Self {
        let mut chunks = HashMap::new();
        for cx in -radius..radius {
            for cz in -radius..radius {
                let mut data = vec![AIR; (CS * CS * CH) as usize];
                for lz in 0..CS {
                    for lx in 0..CS {
                        let h = height_at(cx * CS + lx, cz * CS + lz);
                        for y in 0..CH {
                            data[idx(lx, y, lz)] = column_block(h, y);
                        }
                    }
                }
                chunks.insert((cx, cz), data);
            }
        }
        Self { chunks }
    }

    pub fn chunk_coords(&self) -> Vec<(i32, i32)> {
        let mut v: Vec<_> = self.chunks.keys().copied().collect();
        v.sort();
        v
    }

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

    pub fn mesh_chunk(&self, cx: i32, cz: i32) -> MeshData {
        let mut m = MeshData::default();
        for y in 0..CH {
            for lz in 0..CS {
                for lx in 0..CS {
                    let (x, z) = (cx * CS + lx, cz * CS + lz);
                    let b = self.get(x, y, z);
                    if b == AIR {
                        continue;
                    }
                    let base = block_color(b);
                    // Biến thiên nhẹ theo vị trí cho đỡ trơn.
                    let var = 0.92 + 0.08 * hash2(x + y * 31, z + y * 17);
                    for f in &FACES {
                        if self.get(x + f.n[0], y + f.n[1], z + f.n[2]) != AIR {
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
                        m.indices
                            .extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
                    }
                }
            }
        }
        m
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
        let w = World::new(2);
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
    fn top_faces_only_on_surface() {
        let w = World::new(1);
        let h = height_at(3, 3);
        assert_ne!(w.get(3, h, 3), AIR);
        assert_eq!(w.get(3, h + 1, 3), AIR);
    }
}
