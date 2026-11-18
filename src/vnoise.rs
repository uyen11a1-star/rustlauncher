//! Nhiễu của Minecraft 1.18+: ImprovedNoise, PerlinNoise nhiều tầng và NormalNoise (hai Perlin lệch tần số).
//! Dùng số thực 64 bit đúng như mã Java.
use crate::vrng::Xoro;

const GRADIENT: [[f64; 3]; 16] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
    [1.0, 1.0, 0.0],
    [0.0, -1.0, 1.0],
    [-1.0, 1.0, 0.0],
    [0.0, -1.0, -1.0],
];

pub fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

fn lerp2(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> f64 {
    lerp(b, lerp(a, c, d), lerp(a, e, f))
}

#[allow(clippy::too_many_arguments)]
fn lerp3(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64, g: f64, h: f64, i: f64, j: f64, k: f64) -> f64 {
    lerp(c, lerp2(a, b, d, e, f, g), lerp2(a, b, h, i, j, k))
}

fn smooth(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn grad_dot(h: i32, x: f64, y: f64, z: f64) -> f64 {
    let g = &GRADIENT[(h & 15) as usize];
    g[0] * x + g[1] * y + g[2] * z
}

/// Cuộn tọa độ về khoảng nhỏ để không mất độ chính xác khi ở rất xa gốc.
pub fn wrap(v: f64) -> f64 {
    v - (v / 3.3554432E7 + 0.5).floor() * 3.3554432E7
}

pub struct ImprovedNoise {
    p: [u8; 256],
    xo: f64,
    yo: f64,
    zo: f64,
}

impl ImprovedNoise {
    pub fn new(r: &mut Xoro) -> ImprovedNoise {
        let xo = r.next_f64() * 256.0;
        let yo = r.next_f64() * 256.0;
        let zo = r.next_f64() * 256.0;
        let mut p = [0u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        for i in 0..256usize {
            let k = r.next_bounded(256 - i as i32) as usize;
            p.swap(i, i + k);
        }
        ImprovedNoise { p, xo, yo, zo }
    }

    fn perm(&self, i: i32) -> i32 {
        self.p[(i & 255) as usize] as i32
    }

    pub fn noise(&self, x: f64, y: f64, z: f64) -> f64 {
        self.noise_y(x, y, z, 0.0, 0.0)
    }

    pub fn noise_y(&self, x: f64, y: f64, z: f64, y_scale: f64, y_fudge: f64) -> f64 {
        let (d, e, f) = (x + self.xo, y + self.yo, z + self.zo);
        let (i, j, k) = (d.floor() as i32, e.floor() as i32, f.floor() as i32);
        let (g, h, l) = (d - i as f64, e - j as f64, f - k as f64);
        let o = if y_scale != 0.0 {
            let m = if y_fudge >= 0.0 && y_fudge < h { y_fudge } else { h };
            (m / y_scale + 1.0E-7f32 as f64).floor() * y_scale
        } else {
            0.0
        };
        self.sample_and_lerp(i, j, k, g, h - o, l, h)
    }

    #[allow(clippy::too_many_arguments)]
    fn sample_and_lerp(&self, a: i32, b: i32, c: i32, d: f64, e: f64, f: f64, g: f64) -> f64 {
        let h = self.perm(a);
        let i = self.perm(a + 1);
        let j = self.perm(h + b);
        let k = self.perm(h + b + 1);
        let l = self.perm(i + b);
        let m = self.perm(i + b + 1);
        let n = grad_dot(self.perm(j + c), d, e, f);
        let o = grad_dot(self.perm(l + c), d - 1.0, e, f);
        let q = grad_dot(self.perm(k + c), d, e - 1.0, f);
        let r = grad_dot(self.perm(m + c), d - 1.0, e - 1.0, f);
        let s = grad_dot(self.perm(j + c + 1), d, e, f - 1.0);
        let t = grad_dot(self.perm(l + c + 1), d - 1.0, e, f - 1.0);
        let u = grad_dot(self.perm(k + c + 1), d, e - 1.0, f - 1.0);
        let v = grad_dot(self.perm(m + c + 1), d - 1.0, e - 1.0, f - 1.0);
        lerp3(smooth(d), smooth(g), smooth(f), n, o, q, r, s, t, u, v)
    }
}

/// Nhiễu Perlin nhiều tầng (cách khởi tạo mới của 1.18+).
pub struct Perlin {
    levels: Vec<Option<ImprovedNoise>>,
    amps: Vec<f64>,
    low_in: f64,
    low_val: f64,
}

impl Perlin {
    pub fn new(r: &mut Xoro, first_octave: i32, amps: &[f64]) -> Perlin {
        let n = amps.len();
        let pf = r.fork();
        let mut levels = Vec::with_capacity(n);
        for (l, &a) in amps.iter().enumerate() {
            if a != 0.0 {
                let mut rr = pf.from_hash_of(&format!("octave_{}", first_octave + l as i32));
                levels.push(Some(ImprovedNoise::new(&mut rr)));
            } else {
                levels.push(None);
            }
        }
        Perlin {
            levels,
            amps: amps.to_vec(),
            low_in: 2f64.powi(first_octave),
            low_val: 2f64.powi(n as i32 - 1) / (2f64.powi(n as i32) - 1.0),
        }
    }

    pub fn value(&self, x: f64, y: f64, z: f64) -> f64 {
        let (mut d, mut e, mut f) = (0.0, self.low_in, self.low_val);
        for (i, lv) in self.levels.iter().enumerate() {
            if let Some(n) = lv {
                let g = n.noise(wrap(x * e), wrap(y * e), wrap(z * e));
                d += self.amps[i] * g * f;
            }
            e *= 2.0;
            f /= 2.0;
        }
        d
    }
}

/// NormalNoise: hai Perlin, mẫu thứ hai lệch tần số 1.0181268882175227.
pub struct Normal {
    a: Perlin,
    b: Perlin,
    factor: f64,
}

impl Normal {
    pub fn new(r: &mut Xoro, first_octave: i32, amps: &[f64]) -> Normal {
        let a = Perlin::new(r, first_octave, amps);
        let b = Perlin::new(r, first_octave, amps);
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for (i, &v) in amps.iter().enumerate() {
            if v != 0.0 {
                lo = lo.min(i as i32);
                hi = hi.max(i as i32);
            }
        }
        let span = hi - lo;
        let factor = 0.16666666666666666 / (0.1 * (1.0 + 1.0 / (span + 1) as f64));
        Normal { a, b, factor }
    }

    pub fn value(&self, x: f64, y: f64, z: f64) -> f64 {
        const F: f64 = 1.0181268882175227;
        (self.a.value(x, y, z) + self.b.value(x * F, y * F, z * F)) * self.factor
    }
}
