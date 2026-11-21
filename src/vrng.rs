//! Nền cho sinh địa hình đúng seed Minecraft 1.18+: MD5, trộn Stafford13, Xoroshiro128++ và bộ chia vị trí.
use std::path::Path;

/// MD5 (RFC 1321). Minecraft dùng nó để biến tên nhiễu (vd "minecraft:terrain") thành hạt giống.
pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16,
        23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let mut k = [0u32; 64];
    for (i, v) in k.iter_mut().enumerate() {
        *v = ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32;
    }
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.chunks(64) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64usize {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

pub fn stafford13(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Xoroshiro128++ như `Xoroshiro128PlusPlusRandom` của Minecraft.
#[derive(Clone)]
pub struct Xoro {
    lo: u64,
    hi: u64,
}

impl Xoro {
    pub fn new(lo: u64, hi: u64) -> Xoro {
        if lo | hi == 0 {
            Xoro { lo: 0x9E37_79B9_7F4A_7C15, hi: 0x6A09_E667_F3BC_C909 }
        } else {
            Xoro { lo, hi }
        }
    }

    /// Từ hạt giống 64 bit của thế giới (nâng lên 128 bit bằng Stafford13).
    pub fn from_seed(seed: i64) -> Xoro {
        let l = (seed as u64) ^ 0x6A09_E667_F3BC_C909;
        let m = l.wrapping_add(0x9E37_79B9_7F4A_7C15);
        Xoro::new(stafford13(l), stafford13(m))
    }

    pub fn next_u64(&mut self) -> u64 {
        let (l, m) = (self.lo, self.hi);
        let n = l.wrapping_add(m).rotate_left(17).wrapping_add(l);
        let m = m ^ l;
        self.lo = l.rotate_left(49) ^ m ^ (m << 21);
        self.hi = m.rotate_left(28);
        n
    }

    pub fn next_i32(&mut self) -> i32 {
        self.next_u64() as i32
    }

    /// `nextInt(bound)` của Minecraft (phương pháp nhân, loại mẫu lệch).
    pub fn next_bounded(&mut self, bound: i32) -> i32 {
        let b = bound as u64;
        let mut l = (self.next_i32() as u64) & 0xFFFF_FFFF;
        let mut m = l.wrapping_mul(b);
        let mut n = m & 0xFFFF_FFFF;
        if n < b {
            let i = ((!bound).wrapping_add(1) as u32 as u64) % b;
            while n < i {
                l = (self.next_i32() as u64) & 0xFFFF_FFFF;
                m = l.wrapping_mul(b);
                n = m & 0xFFFF_FFFF;
            }
        }
        (m >> 32) as i32
    }

    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9007199254740992.0)
    }

    /// Bỏ qua `n` số.
    pub fn skip(&mut self, n: usize) {
        for _ in 0..n {
            self.next_u64();
        }
    }

    /// `forkPositional()`: lấy hai số làm gốc cho bộ chia.
    pub fn fork(&mut self) -> Factory {
        let lo = self.next_u64();
        let hi = self.next_u64();
        Factory { lo, hi }
    }
}

/// Bộ chia số ngẫu nhiên theo tên hoặc theo vị trí.
#[derive(Clone)]
pub struct Factory {
    lo: u64,
    hi: u64,
}

impl Factory {
    /// `fromHashOf(name)`: MD5 của tên, hai nửa big-endian, XOR với gốc.
    pub fn from_hash_of(&self, name: &str) -> Xoro {
        let h = md5(name.as_bytes());
        let l = u64::from_be_bytes([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]]);
        let m = u64::from_be_bytes([h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]]);
        Xoro::new(l ^ self.lo, m ^ self.hi)
    }
}

/// Hạt giống thế giới trong level.dat (`Data.WorldGenSettings.seed`, hoặc `Data.RandomSeed` ở bản cũ).
pub fn read_seed(dir: &Path) -> Option<i64> {
    let bytes = std::fs::read(dir.join("level.dat")).ok()?;
    let raw = crate::inflate::gzip_any(&bytes, 64 << 20).ok()?;
    let root = crate::nbt::parse(&raw).ok()?;
    let data = root.get("Data")?;
    data.get("WorldGenSettings").and_then(|w| w.get("seed")).and_then(|s| s.as_i64()).or_else(|| data.get("RandomSeed").and_then(|s| s.as_i64()))
}

/// So với giá trị đã được kiểm với mã Java gốc (lấy từ bộ test của một bản cài đặt khác).
pub fn selftest(log: &dyn Fn(String)) {
    let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let md5_ok = hex(md5(b"abc")) == "900150983cd24fb0d6963f7d28e17f72" && hex(md5(b"")) == "d41d8cd98f00b204e9800998ecf8427e";
    let st_ok = stafford13(1) as i64 == 6238072747940578789 && stafford13(64) as i64 == -8456553050427055661 && stafford13(2) as i64 == -2606959012126976886;
    let want = [-160476802, 781697906, 653572596, 1337520923, -505875771, -47281585, 342195906, 1417498593, -1478887443, 1560080270];
    let mut r = Xoro::from_seed(0);
    let xo_ok = want.iter().all(|&w| r.next_i32() == w);
    let sh = sha256(b"abc");
    let sha_ok = sh[0] == 0xba && sh[1] == 0x78 && sh[2] == 0x16 && sh[3] == 0xbf && sh[31] == 0xad;
    let s = |b: bool| if b { "ĐÚNG" } else { "SAI" };
    log(format!("Kiểm tra nền sinh địa hình: MD5 {}, Stafford13 {}, Xoroshiro {}, SHA-256 {}", s(md5_ok), s(st_ok), s(xo_ok), s(sha_ok)));
}

/// `Mth.getSeed(x, y, z)`: băm vị trí khối thành số 64 bit.
pub fn hash_block_pos(x: i32, y: i32, z: i32) -> i64 {
    let l = ((x.wrapping_mul(3129871)) as i64) ^ ((z as i64).wrapping_mul(116129781i64)) ^ (y as i64);
    let l = l.wrapping_mul(l).wrapping_mul(42317861i64).wrapping_add(l.wrapping_mul(11i64));
    l >> 16
}

impl Xoro {
    /// `nextFloat()`: 24 bit cao.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * 5.960_464_5E-8
    }

    pub fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 != 0
    }

    /// `nextIntBetweenInclusive(a, b)`.
    pub fn next_between(&mut self, a: i32, b: i32) -> i32 {
        self.next_bounded(b - a + 1) + a
    }
}

impl Factory {
    /// `at(x, y, z)`: số ngẫu nhiên gắn với một vị trí.
    pub fn at(&self, x: i32, y: i32, z: i32) -> Xoro {
        Xoro::new((hash_block_pos(x, y, z) as u64) ^ self.lo, self.hi)
    }
}

/// SHA-256 (FIPS 180-4), dùng để làm mờ hạt giống khi chọn quần xã theo khối.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74,
        0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d,
        0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e,
        0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
        0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

/// Hạt giống làm mờ khi chọn quần xã theo khối (`BiomeManager.obfuscateSeed`).
pub fn zoom_seed(seed: i64) -> i64 {
    let h = sha256(&seed.to_le_bytes());
    i64::from_le_bytes([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]])
}
