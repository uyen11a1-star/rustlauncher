//! Hàm mật độ của Minecraft 1.21: đọc JSON thật trong client.jar, dựng cây rồi tính từng khối.
//! Mô phỏng đúng: flat_cache lấy mẫu theo ô 4x4, interpolated nội suy 3 chiều theo ô 4x8x4, spline dùng số 32 bit.
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use serde_json::Value;

use crate::{
    assets::Jar,
    vnoise::{lerp, wrap, ImprovedNoise, Normal},
    vrng::{Factory, Xoro},
};

const NX: usize = 5;
const NY: usize = 49;
const NZ: usize = 5;

pub struct Ctx {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub corner: bool,
}

enum Sp {
    C(f32),
    M { coord: Rc<Df>, locs: Vec<f32>, vals: Vec<Sp>, ders: Vec<f32> },
}

enum Df {
    C(f64),
    Add(Rc<Df>, Rc<Df>),
    Mul(Rc<Df>, Rc<Df>),
    Min(Rc<Df>, Rc<Df>),
    Max(Rc<Df>, Rc<Df>),
    Abs(Rc<Df>),
    Square(Rc<Df>),
    Cube(Rc<Df>),
    HalfNeg(Rc<Df>),
    QuarterNeg(Rc<Df>),
    Squeeze(Rc<Df>),
    Invert(Rc<Df>),
    Clamp(Rc<Df>, f64, f64),
    YGrad { fy: f64, ty: f64, fv: f64, tv: f64 },
    Range { input: Rc<Df>, lo: f64, hi: f64, inr: Rc<Df>, out: Rc<Df> },
    Noise { n: usize, xz: f64, y: f64 },
    Shifted { n: usize, sx: Rc<Df>, sy: Rc<Df>, sz: Rc<Df>, xz: f64, y: f64 },
    ShiftA(usize),
    ShiftB(usize),
    Weird { input: Rc<Df>, n: usize, tunnels: bool },
    Blend(usize),
    Spline(Sp),
    Flat(Rc<Df>, RefCell<HashMap<(i32, i32), f64>>),
    Pass(Rc<Df>),
    Interp(usize, Rc<Df>),
}

/// Bộ ba Perlin kiểu cũ (khởi tạo "legacy") dùng cho old_blended_noise.
struct Legacy {
    levels: Vec<Option<ImprovedNoise>>,
}

impl Legacy {
    fn new(r: &mut Xoro, first: i32, count: usize) -> Legacy {
        let k = (-first) as usize;
        let mut levels: Vec<Option<ImprovedNoise>> = (0..count).map(|_| None).collect();
        levels[k] = Some(ImprovedNoise::new(r));
        for n in (0..k).rev() {
            levels[n] = Some(ImprovedNoise::new(r));
        }
        Legacy { levels }
    }

    fn octave(&self, i: usize) -> Option<&ImprovedNoise> {
        self.levels[self.levels.len() - 1 - i].as_ref()
    }
}

struct Blended {
    min: Legacy,
    max: Legacy,
    main: Legacy,
    xz_mult: f64,
    y_mult: f64,
    xz_factor: f64,
    y_factor: f64,
    smear: f64,
}

impl Blended {
    fn compute(&self, bx: i32, by: i32, bz: i32) -> f64 {
        let d = bx as f64 * self.xz_mult;
        let e = by as f64 * self.y_mult;
        let f = bz as f64 * self.xz_mult;
        let g = d / self.xz_factor;
        let h = e / self.y_factor;
        let i = f / self.xz_factor;
        let j = self.y_mult * self.smear;
        let k = j / self.y_factor;
        let mut n = 0.0;
        let mut o = 1.0;
        for p in 0..8 {
            if let Some(nz) = self.main.octave(p) {
                n += nz.noise_y(wrap(g * o), wrap(h * o), wrap(i * o), k * o, h * o) / o;
            }
            o /= 2.0;
        }
        let q = (n / 10.0 + 1.0) / 2.0;
        let (bl2, bl3) = (q >= 1.0, q <= 0.0);
        let (mut l, mut m) = (0.0, 0.0);
        o = 1.0;
        for r in 0..16 {
            let s = wrap(d * o);
            let t = wrap(e * o);
            let u = wrap(f * o);
            let v = j * o;
            if !bl2 {
                if let Some(nz) = self.min.octave(r) {
                    l += nz.noise_y(s, t, u, v, e * o) / o;
                }
            }
            if !bl3 {
                if let Some(nz) = self.max.octave(r) {
                    m += nz.noise_y(s, t, u, v, e * o) / o;
                }
            }
            o /= 2.0;
        }
        clamped_lerp(l / 512.0, m / 512.0, q) / 128.0
    }
}

fn clamped_lerp(a: f64, b: f64, t: f64) -> f64 {
    if t < 0.0 {
        a
    } else if t > 1.0 {
        b
    } else {
        lerp(t, a, b)
    }
}

fn clamped_map(v: f64, fy: f64, ty: f64, fv: f64, tv: f64) -> f64 {
    clamped_lerp(fv, tv, (v - fy) / (ty - fy))
}

fn lerpf(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

fn linear_extend(x: f32, locs: &[f32], value: f32, ders: &[f32], i: usize) -> f32 {
    let f = ders[i];
    if f == 0.0 {
        value
    } else {
        value + f * (x - locs[i])
    }
}

fn rarity(v: f64, tunnels: bool) -> f64 {
    if tunnels {
        if v < -0.5 {
            0.75
        } else if v < 0.0 {
            1.0
        } else if v < 0.5 {
            1.5
        } else {
            2.0
        }
    } else if v < -0.75 {
        0.5
    } else if v < -0.5 {
        0.75
    } else if v < 0.5 {
        1.0
    } else if v < 0.75 {
        2.0
    } else {
        3.0
    }
}

pub struct Router {
    noises: Vec<Normal>,
    blended: Vec<Blended>,
    interps: Vec<Rc<Df>>,
    grids: RefCell<Vec<Vec<f64>>>,
    origin: Cell<(i32, i32)>,
    min_y: i32,
    final_density: Rc<Df>,
}

struct Comp<'a> {
    jar: &'a Jar,
    fac: Factory,
    noises: Vec<Normal>,
    noise_idx: HashMap<String, usize>,
    blended: Vec<Blended>,
    interps: Vec<Rc<Df>>,
    cache: HashMap<String, Rc<Df>>,
}

fn num(v: &Value, k: &str) -> Result<f64, String> {
    v.get(k).and_then(|x| x.as_f64()).ok_or_else(|| format!("thiếu số '{k}'"))
}

impl<'a> Comp<'a> {
    fn json(&self, path: &str) -> Result<Value, String> {
        let bytes = self.jar.read(path)?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{path}: {e}"))
    }

    fn noise(&mut self, name: &str) -> Result<usize, String> {
        if let Some(&i) = self.noise_idx.get(name) {
            return Ok(i);
        }
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        let v = self.json(&format!("data/minecraft/worldgen/noise/{short}.json"))?;
        let first = v.get("firstOctave").and_then(|x| x.as_i64()).ok_or("noise thiếu firstOctave")? as i32;
        let amps: Vec<f64> = v.get("amplitudes").and_then(|a| a.as_array()).ok_or("noise thiếu amplitudes")?.iter().map(|a| a.as_f64().unwrap_or(0.0)).collect();
        let mut r = self.fac.from_hash_of(&format!("minecraft:{short}"));
        self.noises.push(Normal::new(&mut r, first, &amps));
        let i = self.noises.len() - 1;
        self.noise_idx.insert(name.to_string(), i);
        Ok(i)
    }

    fn named(&mut self, name: &str) -> Result<Rc<Df>, String> {
        if let Some(d) = self.cache.get(name) {
            return Ok(d.clone());
        }
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        let v = self.json(&format!("data/minecraft/worldgen/density_function/{short}.json"))?;
        let d = self.df(&v)?;
        self.cache.insert(name.to_string(), d.clone());
        Ok(d)
    }

    fn arg(&mut self, v: &Value, k: &str) -> Result<Rc<Df>, String> {
        let a = v.get(k).ok_or_else(|| format!("thiếu '{k}'"))?;
        self.df(a)
    }

    fn spline(&mut self, v: &Value) -> Result<Sp, String> {
        if let Some(n) = v.as_f64() {
            return Ok(Sp::C(n as f32));
        }
        let coord = self.df(v.get("coordinate").ok_or("spline thiếu coordinate")?)?;
        let pts = v.get("points").and_then(|p| p.as_array()).ok_or("spline thiếu points")?;
        let (mut locs, mut vals, mut ders) = (Vec::new(), Vec::new(), Vec::new());
        for p in pts {
            locs.push(num(p, "location")? as f32);
            vals.push(self.spline(p.get("value").ok_or("điểm spline thiếu value")?)?);
            ders.push(p.get("derivative").and_then(|d| d.as_f64()).unwrap_or(0.0) as f32);
        }
        Ok(Sp::M { coord, locs, vals, ders })
    }

    fn df(&mut self, v: &Value) -> Result<Rc<Df>, String> {
        match v {
            Value::Number(n) => Ok(Rc::new(Df::C(n.as_f64().unwrap_or(0.0)))),
            Value::String(s) => self.named(s),
            Value::Object(o) => {
                let t = o.get("type").and_then(|t| t.as_str()).ok_or("hàm mật độ thiếu type")?;
                let t = t.strip_prefix("minecraft:").unwrap_or(t);
                let d = match t {
                    "add" => Df::Add(self.arg(v, "argument1")?, self.arg(v, "argument2")?),
                    "mul" => Df::Mul(self.arg(v, "argument1")?, self.arg(v, "argument2")?),
                    "min" => Df::Min(self.arg(v, "argument1")?, self.arg(v, "argument2")?),
                    "max" => Df::Max(self.arg(v, "argument1")?, self.arg(v, "argument2")?),
                    "abs" => Df::Abs(self.arg(v, "argument")?),
                    "square" => Df::Square(self.arg(v, "argument")?),
                    "cube" => Df::Cube(self.arg(v, "argument")?),
                    "half_negative" => Df::HalfNeg(self.arg(v, "argument")?),
                    "quarter_negative" => Df::QuarterNeg(self.arg(v, "argument")?),
                    "squeeze" => Df::Squeeze(self.arg(v, "argument")?),
                    "invert" => Df::Invert(self.arg(v, "argument")?),
                    "clamp" => Df::Clamp(self.arg(v, "input")?, num(v, "min")?, num(v, "max")?),
                    "y_clamped_gradient" => Df::YGrad { fy: num(v, "from_y")?, ty: num(v, "to_y")?, fv: num(v, "from_value")?, tv: num(v, "to_value")? },
                    "range_choice" => Df::Range {
                        input: self.arg(v, "input")?,
                        lo: num(v, "min_inclusive")?,
                        hi: num(v, "max_exclusive")?,
                        inr: self.arg(v, "when_in_range")?,
                        out: self.arg(v, "when_out_of_range")?,
                    },
                    "noise" => {
                        let name = v.get("noise").and_then(|n| n.as_str()).ok_or("noise thiếu tên")?.to_string();
                        Df::Noise { n: self.noise(&name)?, xz: num(v, "xz_scale")?, y: num(v, "y_scale")? }
                    }
                    "shifted_noise" => {
                        let name = v.get("noise").and_then(|n| n.as_str()).ok_or("shifted_noise thiếu tên")?.to_string();
                        Df::Shifted {
                            n: self.noise(&name)?,
                            sx: self.arg(v, "shift_x")?,
                            sy: self.arg(v, "shift_y")?,
                            sz: self.arg(v, "shift_z")?,
                            xz: num(v, "xz_scale")?,
                            y: num(v, "y_scale")?,
                        }
                    }
                    "shift_a" | "shift_b" => {
                        let name = v.get("argument").and_then(|n| n.as_str()).ok_or("shift thiếu tên nhiễu")?.to_string();
                        let n = self.noise(&name)?;
                        if t == "shift_a" {
                            Df::ShiftA(n)
                        } else {
                            Df::ShiftB(n)
                        }
                    }
                    "weird_scaled_sampler" => {
                        let name = v.get("noise").and_then(|n| n.as_str()).ok_or("weird thiếu tên nhiễu")?.to_string();
                        let m = v.get("rarity_value_mapper").and_then(|m| m.as_str()).unwrap_or("type_1");
                        Df::Weird { input: self.arg(v, "input")?, n: self.noise(&name)?, tunnels: m == "type_1" }
                    }
                    "old_blended_noise" => {
                        let mut r = self.fac.from_hash_of("minecraft:terrain");
                        let min = Legacy::new(&mut r, -15, 16);
                        let max = Legacy::new(&mut r, -15, 16);
                        let main = Legacy::new(&mut r, -7, 8);
                        self.blended.push(Blended {
                            min,
                            max,
                            main,
                            xz_mult: 684.412 * num(v, "xz_scale")?,
                            y_mult: 684.412 * num(v, "y_scale")?,
                            xz_factor: num(v, "xz_factor")?,
                            y_factor: num(v, "y_factor")?,
                            smear: num(v, "smear_scale_multiplier")?,
                        });
                        Df::Blend(self.blended.len() - 1)
                    }
                    "spline" => Df::Spline(self.spline(v.get("spline").ok_or("spline thiếu thân")?)?),
                    "flat_cache" => Df::Flat(self.arg(v, "argument")?, RefCell::new(HashMap::new())),
                    "cache_2d" | "cache_once" | "cache_all_in_cell" | "blend_density" => Df::Pass(self.arg(v, "argument")?),
                    "blend_alpha" => Df::C(1.0),
                    "blend_offset" => Df::C(0.0),
                    "interpolated" => {
                        let inner = self.arg(v, "argument")?;
                        self.interps.push(inner.clone());
                        Df::Interp(self.interps.len() - 1, inner)
                    }
                    other => return Err(format!("chưa hỗ trợ hàm mật độ '{other}'")),
                };
                Ok(Rc::new(d))
            }
            _ => Err("hàm mật độ có kiểu JSON lạ".into()),
        }
    }
}

impl Router {
    /// Dựng bộ máy từ dữ liệu thật trong jar cho một hạt giống thế giới.
    pub fn build(jar: &Jar, seed: i64) -> Result<Router, String> {
        let fac = Xoro::from_seed(seed).fork();
        let mut c = Comp { jar, fac, noises: Vec::new(), noise_idx: HashMap::new(), blended: Vec::new(), interps: Vec::new(), cache: HashMap::new() };
        let settings = c.json("data/minecraft/worldgen/noise_settings/overworld.json")?;
        let min_y = settings.get("noise").and_then(|n| n.get("min_y")).and_then(|v| v.as_i64()).ok_or("thiếu noise.min_y")? as i32;
        let fd = settings.get("noise_router").and_then(|r| r.get("final_density")).ok_or("thiếu final_density")?;
        let final_density = c.df(fd)?;
        let n = c.interps.len();
        Ok(Router { noises: c.noises, blended: c.blended, interps: c.interps, grids: RefCell::new(vec![Vec::new(); n]), origin: Cell::new((0, 0)), min_y, final_density })
    }

    /// Tính sẵn lưới các góc ô (4x8x4) cho các nút `interpolated` của chunk có góc tại (x0, z0).
    pub fn prepare(&self, x0: i32, z0: i32) {
        self.origin.set((x0, z0));
        for (i, inner) in self.interps.iter().enumerate() {
            let mut vals = Vec::with_capacity(NX * NY * NZ);
            for ix in 0..NX {
                for iy in 0..NY {
                    for iz in 0..NZ {
                        let c = Ctx { x: x0 + 4 * ix as i32, y: self.min_y + 8 * iy as i32, z: z0 + 4 * iz as i32, corner: true };
                        vals.push(self.eval(inner, &c));
                    }
                }
            }
            self.grids.borrow_mut()[i] = vals;
        }
    }

    /// Mật độ tại một khối; dương nghĩa là đặc. Gọi `prepare` cho chunk chứa khối trước.
    pub fn density(&self, x: i32, y: i32, z: i32) -> f64 {
        self.eval(&self.final_density, &Ctx { x, y, z, corner: false })
    }

    fn sp(&self, s: &Sp, c: &Ctx) -> f32 {
        match s {
            Sp::C(v) => *v,
            Sp::M { coord, locs, vals, ders } => {
                let x = self.eval(coord, c) as f32;
                let n = locs.len();
                let mut i = 0;
                while i < n && !(x < locs[i]) {
                    i += 1;
                }
                if i == 0 {
                    return linear_extend(x, locs, self.sp(&vals[0], c), ders, 0);
                }
                let i = i - 1;
                if i == n - 1 {
                    return linear_extend(x, locs, self.sp(&vals[n - 1], c), ders, n - 1);
                }
                let (g, h) = (locs[i], locs[i + 1]);
                let k = (x - g) / (h - g);
                let (l, m) = (ders[i], ders[i + 1]);
                let a = self.sp(&vals[i], c);
                let b = self.sp(&vals[i + 1], c);
                let p = l * (h - g) - (b - a);
                let q = -m * (h - g) + (b - a);
                lerpf(k, a, b) + k * (1.0 - k) * lerpf(k, p, q)
            }
        }
    }

    fn eval(&self, d: &Df, c: &Ctx) -> f64 {
        match d {
            Df::C(v) => *v,
            Df::Add(a, b) => self.eval(a, c) + self.eval(b, c),
            Df::Mul(a, b) => {
                let x = self.eval(a, c);
                if x == 0.0 {
                    0.0
                } else {
                    x * self.eval(b, c)
                }
            }
            Df::Min(a, b) => {
                let (x, y) = (self.eval(a, c), self.eval(b, c));
                x.min(y)
            }
            Df::Max(a, b) => {
                let (x, y) = (self.eval(a, c), self.eval(b, c));
                x.max(y)
            }
            Df::Abs(a) => self.eval(a, c).abs(),
            Df::Square(a) => {
                let v = self.eval(a, c);
                v * v
            }
            Df::Cube(a) => {
                let v = self.eval(a, c);
                v * v * v
            }
            Df::HalfNeg(a) => {
                let v = self.eval(a, c);
                if v > 0.0 {
                    v
                } else {
                    v * 0.5
                }
            }
            Df::QuarterNeg(a) => {
                let v = self.eval(a, c);
                if v > 0.0 {
                    v
                } else {
                    v * 0.25
                }
            }
            Df::Squeeze(a) => {
                let v = self.eval(a, c).clamp(-1.0, 1.0);
                v / 2.0 - v * v * v / 24.0
            }
            Df::Invert(a) => 1.0 / self.eval(a, c),
            Df::Clamp(a, lo, hi) => {
                let v = self.eval(a, c);
                if v < *lo {
                    *lo
                } else if v > *hi {
                    *hi
                } else {
                    v
                }
            }
            Df::YGrad { fy, ty, fv, tv } => clamped_map(c.y as f64, *fy, *ty, *fv, *tv),
            Df::Range { input, lo, hi, inr, out } => {
                let v = self.eval(input, c);
                if v >= *lo && v < *hi {
                    self.eval(inr, c)
                } else {
                    self.eval(out, c)
                }
            }
            Df::Noise { n, xz, y } => self.noises[*n].value(c.x as f64 * xz, c.y as f64 * y, c.z as f64 * xz),
            Df::Shifted { n, sx, sy, sz, xz, y } => {
                let a = c.x as f64 * xz + self.eval(sx, c);
                let b = c.y as f64 * y + self.eval(sy, c);
                let e = c.z as f64 * xz + self.eval(sz, c);
                self.noises[*n].value(a, b, e)
            }
            Df::ShiftA(n) => self.noises[*n].value(c.x as f64 * 0.25, 0.0, c.z as f64 * 0.25) * 4.0,
            Df::ShiftB(n) => self.noises[*n].value(c.z as f64 * 0.25, c.x as f64 * 0.25, 0.0) * 4.0,
            Df::Weird { input, n, tunnels } => {
                let r = rarity(self.eval(input, c), *tunnels);
                r * self.noises[*n].value(c.x as f64 / r, c.y as f64 / r, c.z as f64 / r).abs()
            }
            Df::Blend(i) => self.blended[*i].compute(c.x, c.y, c.z),
            Df::Spline(s) => self.sp(s, c) as f64,
            Df::Flat(inner, memo) => {
                let key = (c.x >> 2, c.z >> 2);
                if let Some(v) = memo.borrow().get(&key) {
                    return *v;
                }
                let cc = Ctx { x: key.0 << 2, y: 0, z: key.1 << 2, corner: c.corner };
                let v = self.eval(inner, &cc);
                memo.borrow_mut().insert(key, v);
                v
            }
            Df::Pass(a) => self.eval(a, c),
            Df::Interp(i, inner) => {
                if c.corner {
                    return self.eval(inner, c);
                }
                let (ox, oz) = self.origin.get();
                let (lx, lz) = (c.x - ox, c.z - oz);
                let ly = c.y - self.min_y;
                let (cx, cy, cz) = ((lx >> 2) as usize, (ly >> 3) as usize, (lz >> 2) as usize);
                let (fx, fy, fz) = ((lx & 3) as f64 / 4.0, (ly & 7) as f64 / 8.0, (lz & 3) as f64 / 4.0);
                let all = self.grids.borrow();
                let g = &all[*i];
                let at = |a: usize, b: usize, d: usize| g[(a * NY + b) * NZ + d];
                let a = lerp(fy, at(cx, cy, cz), at(cx, cy + 1, cz));
                let b = lerp(fy, at(cx + 1, cy, cz), at(cx + 1, cy + 1, cz));
                let e = lerp(fy, at(cx, cy, cz + 1), at(cx, cy + 1, cz + 1));
                let f = lerp(fy, at(cx + 1, cy, cz + 1), at(cx + 1, cy + 1, cz + 1));
                lerp(fz, lerp(fx, a, b), lerp(fx, e, f))
            }
        }
    }
}
