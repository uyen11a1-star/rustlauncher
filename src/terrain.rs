//! Sinh địa hình: nhiễu Perlin 2D, đồi núi, biển, bãi cát, sa mạc, rừng, tuyết, cây, cỏ hoa.
//! Chạy theo từng chunk, cùng hạt giống (SEED) thì cùng kết quả, cây tràn qua ranh giới chunk vẫn khớp.
use std::sync::OnceLock;

use crate::world::{RawChunk, RawSection};

pub const SEED: u64 = 0x5EED_1234_ABCD_0042;
const SEA: i32 = 63;
const MIN_Y: i32 = -64;
/// Lề quanh chunk để xét cây của chunk bên cạnh (tán lá tối đa 2 ô).
const PAD: i32 = 3;
const W: usize = 16 + 2 * PAD as usize;

// Thứ tự phải khớp các hằng số id bên dưới.
const PALETTE: [&str; 18] = [
    "minecraft:air",
    "minecraft:stone",
    "minecraft:deepslate[axis=y]",
    "minecraft:bedrock",
    "minecraft:dirt",
    "minecraft:grass_block[snowy=false]",
    "minecraft:sand",
    "minecraft:sandstone",
    "minecraft:gravel",
    "minecraft:water[level=0]",
    "minecraft:snow_block",
    "minecraft:oak_log[axis=y]",
    "minecraft:oak_leaves[distance=1,persistent=false,waterlogged=false]",
    "minecraft:spruce_log[axis=y]",
    "minecraft:spruce_leaves[distance=1,persistent=false,waterlogged=false]",
    "minecraft:short_grass",
    "minecraft:dandelion",
    "minecraft:poppy",
];
const AIR: u8 = 0;
const STONE: u8 = 1;
const DEEP: u8 = 2;
const BEDROCK: u8 = 3;
const DIRT: u8 = 4;
const GRASS: u8 = 5;
const SAND: u8 = 6;
const SANDSTONE: u8 = 7;
const GRAVEL: u8 = 8;
const WATER: u8 = 9;
const SNOW: u8 = 10;
const OAK_LOG: u8 = 11;
const OAK_LEAF: u8 = 12;
const SPR_LOG: u8 = 13;
const SPR_LEAF: u8 = 14;
const TALL_GRASS: u8 = 15;
const DANDELION: u8 = 16;
const POPPY: u8 = 17;

#[derive(Clone, Copy, PartialEq)]
enum Biome {
    Ocean,
    Beach,
    Plains,
    Forest,
    Desert,
    Snow,
    Mount,
}

struct Perlin {
    p: [u8; 512],
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn grad(h: u8, x: f32, y: f32) -> f32 {
    match h & 7 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x,
        5 => -x,
        6 => y,
        _ => -y,
    }
}

impl Perlin {
    fn new(seed: u64) -> Perlin {
        let mut t: Vec<u8> = (0..=255).collect();
        let mut s = seed | 1;
        for i in (1..256).rev() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let j = (s % (i as u64 + 1)) as usize;
            t.swap(i, j);
        }
        let mut p = [0u8; 512];
        for i in 0..512 {
            p[i] = t[i & 255];
        }
        Perlin { p }
    }

    fn at(&self, x: f32, y: f32) -> f32 {
        let (fx, fy) = (x.floor(), y.floor());
        let xi = (fx as i32 & 255) as usize;
        let yi = (fy as i32 & 255) as usize;
        let (xf, yf) = (x - fx, y - fy);
        let (u, v) = (fade(xf), fade(yf));
        let p = &self.p;
        let aa = p[p[xi] as usize + yi];
        let ab = p[p[xi] as usize + yi + 1];
        let ba = p[p[xi + 1] as usize + yi];
        let bb = p[p[xi + 1] as usize + yi + 1];
        lerp(
            lerp(grad(aa, xf, yf), grad(ba, xf - 1.0, yf), u),
            lerp(grad(ab, xf, yf - 1.0), grad(bb, xf - 1.0, yf - 1.0), u),
            v,
        )
    }

    /// Cộng dồn các tầng nhiễu, chuẩn hóa về khoảng gần [-1, 1].
    fn fbm(&self, x: f32, y: f32, oct: u32) -> f32 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for _ in 0..oct {
            sum += self.at(x * freq, y * freq) * amp;
            norm += amp;
            amp *= 0.5;
            freq *= 2.0;
        }
        sum / norm * 1.6
    }
}

struct Noises {
    cont: Perlin,
    hill: Perlin,
    ridge: Perlin,
    temp: Perlin,
    humid: Perlin,
}

fn noises() -> &'static Noises {
    static N: OnceLock<Noises> = OnceLock::new();
    N.get_or_init(|| Noises {
        cont: Perlin::new(SEED ^ 0x1111),
        hill: Perlin::new(SEED ^ 0x2222),
        ridge: Perlin::new(SEED ^ 0x3333),
        temp: Perlin::new(SEED ^ 0x4444),
        humid: Perlin::new(SEED ^ 0x5555),
    })
}

fn hash(a: i32, b: i32, salt: u64) -> u64 {
    let mut h = SEED ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= (a as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = h.rotate_left(27).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= (b as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^ (h >> 29)
}

/// Độ cao mặt đất và quần xã tại cột (x, z).
fn column(x: i32, z: i32) -> (i32, Biome) {
    let n = noises();
    let (fx, fz) = (x as f32, z as f32);
    let cont = n.cont.fbm(fx / 400.0, fz / 400.0, 3);
    let hills = n.hill.fbm(fx / 120.0, fz / 120.0, 4);
    let ridge = 1.0 - n.ridge.fbm(fx / 220.0, fz / 220.0, 3).abs().min(1.0);
    let mount = (cont - 0.15).max(0.0) * 2.0 * ridge * ridge * 60.0;
    let h = (66.0 + cont * 28.0 + hills * 10.0 + mount).round() as i32;
    let temp = n.temp.fbm(fx / 500.0, fz / 500.0, 2);
    let humid = n.humid.fbm(fx / 400.0, fz / 400.0, 2);
    let b = if h < SEA - 1 {
        Biome::Ocean
    } else if h <= SEA + 1 {
        Biome::Beach
    } else if h > 120 {
        Biome::Mount
    } else if temp > 0.25 && humid < 0.0 {
        Biome::Desert
    } else if temp < -0.3 {
        Biome::Snow
    } else if humid > 0.1 {
        Biome::Forest
    } else {
        Biome::Plains
    };
    (h, b)
}

/// Sinh một chunk (dạng chunk thô, cùng định dạng với chunk đọc từ file thế giới).
pub fn chunk(cx: i32, cz: i32) -> RawChunk {
    let (x0, z0) = (cx * 16, cz * 16);
    // độ cao và quần xã của vùng 16x16 cộng lề
    let mut hs = [[0i32; W]; W];
    let mut bs = [[Biome::Plains; W]; W];
    let mut maxh = SEA;
    for j in 0..W {
        for i in 0..W {
            let (h, b) = column(x0 - PAD + i as i32, z0 - PAD + j as i32);
            hs[j][i] = h;
            bs[j][i] = b;
            maxh = maxh.max(h);
        }
    }
    let max_y = maxh + 14;
    let ny = (max_y - MIN_Y + 1) as usize;
    let mut blk = vec![AIR; ny * 256];
    let at = |x: usize, y: i32, z: usize| ((y - MIN_Y) as usize) * 256 + z * 16 + x;

    // địa hình
    for z in 0..16usize {
        for x in 0..16usize {
            let (h, b) = (hs[z + PAD as usize][x + PAD as usize], bs[z + PAD as usize][x + PAD as usize]);
            let (wx, wz) = (x0 + x as i32, z0 + z as i32);
            for y in MIN_Y..=h.max(SEA) {
                let id = if y > h {
                    WATER
                } else if y == MIN_Y {
                    BEDROCK
                } else if y < MIN_Y + 4 && hash(wx * 7 + y, wz, 1) % 3 == 0 {
                    BEDROCK
                } else if y < 0 {
                    DEEP
                } else {
                    let d = h - y; // độ sâu dưới mặt đất
                    match b {
                        Biome::Plains | Biome::Forest => match d {
                            0 => GRASS,
                            1..=3 => DIRT,
                            _ => STONE,
                        },
                        Biome::Snow => match d {
                            0 => SNOW,
                            1..=3 => DIRT,
                            _ => STONE,
                        },
                        Biome::Desert | Biome::Beach => match d {
                            0..=3 => SAND,
                            4..=6 => SANDSTONE,
                            _ => STONE,
                        },
                        Biome::Ocean => match d {
                            0..=2 => {
                                if hash(wx >> 3, wz >> 3, 2) % 3 == 0 {
                                    GRAVEL
                                } else {
                                    SAND
                                }
                            }
                            _ => STONE,
                        },
                        Biome::Mount => {
                            if d == 0 && h > 150 {
                                SNOW
                            } else {
                                STONE
                            }
                        }
                    }
                };
                blk[at(x, y, z)] = id;
            }
        }
    }

    // cây: xét cả cột lân cận trong lề để tán lá tràn qua ranh giới chunk
    for j in 0..W {
        for i in 0..W {
            let (h, b) = (hs[j][i], bs[j][i]);
            if h <= SEA {
                continue;
            }
            let (wx, wz) = (x0 - PAD + i as i32, z0 - PAD + j as i32);
            let r = hash(wx, wz, 3) % 1000;
            let (log, leaf, th) = match b {
                Biome::Forest if r < 45 => (OAK_LOG, OAK_LEAF, 4 + (hash(wx, wz, 4) % 3) as i32),
                Biome::Plains if r < 4 => (OAK_LOG, OAK_LEAF, 4 + (hash(wx, wz, 4) % 3) as i32),
                Biome::Snow if r < 18 => (SPR_LOG, SPR_LEAF, 6 + (hash(wx, wz, 4) % 3) as i32),
                _ => continue,
            };
            let base = h + 1;
            let mut put = |x: i32, y: i32, z: i32, id: u8, over_leaf: bool| {
                let (lx, lz) = (x - x0, z - z0);
                if !(0..16).contains(&lx) || !(0..16).contains(&lz) || y < MIN_Y || y > max_y {
                    return;
                }
                let k = at(lx as usize, y, lz as usize);
                if blk[k] == AIR || (over_leaf && (blk[k] == OAK_LEAF || blk[k] == SPR_LEAF)) {
                    blk[k] = id;
                }
            };
            if log == OAK_LOG {
                for y in base + th - 2..=base + th + 1 {
                    let rad = if y >= base + th { 1 } else { 2 };
                    for dz in -rad..=rad {
                        for dx in -rad..=rad {
                            let corner = dx.abs() == rad && dz.abs() == rad;
                            if corner && (rad == 1 || hash(wx + dx, wz + dz + y, 5) % 2 == 0) {
                                continue;
                            }
                            put(wx + dx, y, wz + dz, leaf, false);
                        }
                    }
                }
            } else {
                for y in base + 2..=base + th + 1 {
                    let rad = if y == base + th + 1 {
                        0
                    } else if (y - base) % 2 == 0 {
                        2
                    } else {
                        1
                    };
                    for dz in -rad..=rad {
                        for dx in -rad..=rad {
                            if dx.abs() + dz.abs() > rad + 1 {
                                continue;
                            }
                            put(wx + dx, y, wz + dz, leaf, false);
                        }
                    }
                }
            }
            for y in base..base + th {
                put(wx, y, wz, log, true);
            }
        }
    }

    // cỏ và hoa dại trên mặt cỏ
    for z in 0..16usize {
        for x in 0..16usize {
            let (h, b) = (hs[z + PAD as usize][x + PAD as usize], bs[z + PAD as usize][x + PAD as usize]);
            if h <= SEA || !(b == Biome::Plains || b == Biome::Forest) {
                continue;
            }
            let (wx, wz) = (x0 + x as i32, z0 + z as i32);
            let r = hash(wx, wz, 6) % 100;
            let id = if r < 14 {
                TALL_GRASS
            } else if r == 14 {
                DANDELION
            } else if r == 15 {
                POPPY
            } else {
                continue;
            };
            let k = at(x, h + 1, z);
            if blk[k] == AIR {
                blk[k] = id;
            }
        }
    }

    // cắt thành các section, bỏ section toàn không khí
    let palette: Vec<String> = PALETTE.iter().map(|s| s.to_string()).collect();
    let mut secs = Vec::new();
    for sy in (MIN_Y >> 4)..=(max_y >> 4) {
        let mut idx = vec![0u16; 4096];
        let mut any = false;
        for ly in 0..16i32 {
            let y = sy * 16 + ly;
            if y < MIN_Y || y > max_y {
                continue;
            }
            for zx in 0..256usize {
                let id = blk[((y - MIN_Y) as usize) * 256 + zx];
                if id != AIR {
                    any = true;
                }
                idx[(ly as usize) * 256 + zx] = id as u16;
            }
        }
        if any {
            secs.push(RawSection { y: sy, palette: palette.clone(), idx: Some(idx) });
        }
    }
    RawChunk { cx, cz, secs, lights: Vec::new() }
}
