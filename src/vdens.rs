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
    vbiome::BiomeSource,
    vnoise::{lerp, wrap, ImprovedNoise, Normal},
    vrng::{zoom_seed, Factory, Xoro},
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
    TopSurface { density: Rc<Df>, upper: Rc<Df>, lower: i32, cell: i32 },
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
    climate: Vec<Rc<Df>>,
    prelim: Rc<Df>,
    rule: Rule,
    names: Vec<String>,
    bands: Vec<u16>,
    surf_noise: usize,
    sec_noise: usize,
    band_noise: usize,
    fac: Factory,
    zoom: i64,
    bsrc: BiomeSource,
    temps: HashMap<&'static str, f32>,
    biome_memo: RefCell<HashMap<(i32, i32, i32), &'static str>>,
    prelim_memo: RefCell<HashMap<(i32, i32), i32>>,
    sea: i32,
    height: i32,
}

struct Comp<'a> {
    jar: &'a Jar,
    fac: Factory,
    noises: Vec<Normal>,
    noise_idx: HashMap<String, usize>,
    blended: Vec<Blended>,
    interps: Vec<Rc<Df>>,
    cache: HashMap<String, Rc<Df>>,
    surf_names: Vec<String>,
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
                    "find_top_surface" => Df::TopSurface {
                        density: self.arg(v, "density")?,
                        upper: self.arg(v, "upper_bound")?,
                        lower: num(v, "lower_bound")? as i32,
                        cell: num(v, "cell_height")? as i32,
                    },
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
        let base = fac.clone();
        let mut c = Comp {
            jar,
            fac,
            noises: Vec::new(),
            noise_idx: HashMap::new(),
            blended: Vec::new(),
            interps: Vec::new(),
            cache: HashMap::new(),
            surf_names: vec!["air".to_string(), "water".to_string(), "stone".to_string()],
        };
        let settings = c.json("data/minecraft/worldgen/noise_settings/overworld.json")?;
        let min_y = settings.get("noise").and_then(|n| n.get("min_y")).and_then(|v| v.as_i64()).ok_or("thiếu noise.min_y")? as i32;
        let fd = settings.get("noise_router").and_then(|r| r.get("final_density")).ok_or("thiếu final_density")?;
        let final_density = c.df(fd)?;
        let nr = settings.get("noise_router").ok_or("thiếu noise_router")?;
        let mut climate = Vec::new();
        for k in ["temperature", "vegetation", "continents", "erosion", "depth", "ridges"] {
            climate.push(c.df(nr.get(k).ok_or_else(|| format!("router thiếu '{k}'"))?)?);
        }
        let prelim = c.df(nr.get("preliminary_surface_level").ok_or("router thiếu preliminary_surface_level")?)?;
        let height = settings.get("noise").and_then(|n| n.get("height")).and_then(|v| v.as_i64()).ok_or("thiếu noise.height")? as i32;
        let sea = settings.get("sea_level").and_then(|v| v.as_i64()).ok_or("thiếu sea_level")? as i32;
        let rule = c.rule(settings.get("surface_rule").ok_or("thiếu surface_rule")?)?;
        let surf_noise = c.noise("minecraft:surface")?;
        let sec_noise = c.noise("minecraft:surface_secondary")?;
        let band_noise = c.noise("minecraft:clay_bands_offset")?;
        let bands = c.clay_bands();
        let bsrc = BiomeSource::new();
        let mut temps: HashMap<&'static str, f32> = HashMap::new();
        for name in bsrc.names() {
            let t = c.json(&format!("data/minecraft/worldgen/biome/{name}.json")).ok().and_then(|b| b.get("temperature").and_then(|t| t.as_f64())).unwrap_or(0.5);
            temps.insert(name, t as f32);
        }
        let n = c.interps.len();
        Ok(Router {
            noises: c.noises,
            blended: c.blended,
            interps: c.interps,
            grids: RefCell::new(vec![Vec::new(); n]),
            origin: Cell::new((0, 0)),
            min_y,
            final_density,
            climate,
            prelim,
            rule,
            names: c.surf_names,
            bands,
            surf_noise,
            sec_noise,
            band_noise,
            fac: base,
            zoom: zoom_seed(seed),
            bsrc,
            temps,
            biome_memo: RefCell::new(HashMap::new()),
            prelim_memo: RefCell::new(HashMap::new()),
            sea,
            height,
        })
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

    /// Điểm khí hậu (đã lượng tử hóa) tại ô quần xã (đơn vị 4 khối): nhiệt độ, độ ẩm, lục địa, xói mòn, độ sâu, độ kỳ quặc.
    pub fn climate(&self, qx: i32, qy: i32, qz: i32) -> [i64; 6] {
        let c = Ctx { x: qx << 2, y: qy << 2, z: qz << 2, corner: false };
        let mut o = [0i64; 6];
        for i in 0..6 {
            let v = self.eval(&self.climate[i], &c);
            o[i] = ((v as f32) * 10000.0f32) as i64;
        }
        o
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
            Df::TopSurface { density, upper, lower, cell } => {
                let u = self.eval(upper, c);
                let mut y = ((u / *cell as f64).floor() as i32) * *cell;
                while y >= *lower {
                    let cc = Ctx { x: c.x, y, z: c.z, corner: true };
                    if self.eval(density, &cc) > 0.0 {
                        return y as f64;
                    }
                    y -= *cell;
                }
                *lower as f64
            }
            Df::Interp(i, inner) => {
                if c.corner || self.grids.borrow()[*i].is_empty() {
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

// ---------------------------------------------------------------------------------------------
// Luật bề mặt: lớp đất đá, cát, tuyết... phủ lên địa hình theo quần xã và độ sâu.

#[derive(Clone, Copy)]
enum Anchor {
    Abs(i32),
    Bottom(i32),
    Top(i32),
}

impl Anchor {
    fn y(self, min_y: i32, height: i32) -> i32 {
        match self {
            Anchor::Abs(v) => v,
            Anchor::Bottom(v) => min_y + v,
            Anchor::Top(v) => min_y + height - 1 - v,
        }
    }
}

enum Cond {
    Biome(Vec<String>),
    Noise { n: usize, lo: f64, hi: f64 },
    VGrad { fac: Factory, t: Anchor, f: Anchor },
    YAbove { a: Anchor, mult: i32, add: bool },
    Water { offset: i32, mult: i32, add: bool },
    Temp,
    Steep,
    Hole,
    Prelim,
    Not(Box<Cond>),
    Depth { offset: i32, add: bool, range: i32, ceiling: bool },
}

enum Rule {
    Block(u16),
    Seq(Vec<Rule>),
    If(Cond, Box<Rule>),
    Bands,
}

struct SCtx {
    x: i32,
    y: i32,
    z: i32,
    run_depth: i32,
    secondary: f64,
    stone_above: i32,
    stone_below: i32,
    fluid_h: i32,
    min_surface: i32,
    steep: bool,
}

fn int(v: &Value, k: &str) -> Result<i32, String> {
    v.get(k).and_then(|x| x.as_i64()).map(|x| x as i32).ok_or_else(|| format!("thiếu số nguyên '{k}'"))
}

impl<'a> Comp<'a> {
    fn intern(&mut self, name: &str) -> u16 {
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        if let Some(i) = self.surf_names.iter().position(|n| n == short) {
            return i as u16;
        }
        self.surf_names.push(short.to_string());
        (self.surf_names.len() - 1) as u16
    }

    fn anchor(&self, v: &Value) -> Result<Anchor, String> {
        if let Some(n) = v.get("absolute").and_then(|x| x.as_i64()) {
            Ok(Anchor::Abs(n as i32))
        } else if let Some(n) = v.get("above_bottom").and_then(|x| x.as_i64()) {
            Ok(Anchor::Bottom(n as i32))
        } else if let Some(n) = v.get("below_top").and_then(|x| x.as_i64()) {
            Ok(Anchor::Top(n as i32))
        } else {
            Err("neo độ cao lạ".into())
        }
    }

    fn cond(&mut self, v: &Value) -> Result<Cond, String> {
        let t = v.get("type").and_then(|t| t.as_str()).ok_or("điều kiện thiếu type")?;
        let t = t.strip_prefix("minecraft:").unwrap_or(t);
        let flag = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
        Ok(match t {
            "biome" => {
                let l = v.get("biome_is").and_then(|b| b.as_array()).ok_or("biome thiếu biome_is")?;
                Cond::Biome(l.iter().filter_map(|b| b.as_str().map(|s| s.strip_prefix("minecraft:").unwrap_or(s).to_string())).collect())
            }
            "noise_threshold" => {
                let name = v.get("noise").and_then(|n| n.as_str()).ok_or("noise_threshold thiếu tên")?.to_string();
                Cond::Noise { n: self.noise(&name)?, lo: num(v, "min_threshold")?, hi: num(v, "max_threshold")? }
            }
            "vertical_gradient" => {
                let name = v.get("random_name").and_then(|n| n.as_str()).ok_or("vertical_gradient thiếu tên")?;
                let fac = self.fac.from_hash_of(name).fork();
                Cond::VGrad { fac, t: self.anchor(v.get("true_at_and_below").ok_or("thiếu true_at_and_below")?)?, f: self.anchor(v.get("false_at_and_above").ok_or("thiếu false_at_and_above")?)? }
            }
            "y_above" => Cond::YAbove { a: self.anchor(v.get("anchor").ok_or("y_above thiếu anchor")?)?, mult: int(v, "surface_depth_multiplier")?, add: flag("add_stone_depth") },
            "water" => Cond::Water { offset: int(v, "offset")?, mult: int(v, "surface_depth_multiplier")?, add: flag("add_stone_depth") },
            "temperature" => Cond::Temp,
            "steep" => Cond::Steep,
            "hole" => Cond::Hole,
            "above_preliminary_surface" => Cond::Prelim,
            "not" => Cond::Not(Box::new(self.cond(v.get("invert").ok_or("not thiếu invert")?)?)),
            "stone_depth" => Cond::Depth {
                offset: int(v, "offset")?,
                add: flag("add_surface_depth"),
                range: int(v, "secondary_depth_range")?,
                ceiling: v.get("surface_type").and_then(|s| s.as_str()) == Some("ceiling"),
            },
            other => return Err(format!("chưa hỗ trợ điều kiện bề mặt '{other}'")),
        })
    }

    fn rule(&mut self, v: &Value) -> Result<Rule, String> {
        let t = v.get("type").and_then(|t| t.as_str()).ok_or("luật thiếu type")?;
        let t = t.strip_prefix("minecraft:").unwrap_or(t);
        Ok(match t {
            "block" => {
                let name = v.get("result_state").and_then(|s| s.get("Name")).and_then(|n| n.as_str()).ok_or("block thiếu Name")?.to_string();
                Rule::Block(self.intern(&name))
            }
            "sequence" => {
                let l = v.get("sequence").and_then(|s| s.as_array()).ok_or("sequence thiếu danh sách")?;
                let mut out = Vec::new();
                for r in l {
                    out.push(self.rule(r)?);
                }
                Rule::Seq(out)
            }
            "condition" => {
                let c = self.cond(v.get("if_true").ok_or("condition thiếu if_true")?)?;
                let r = self.rule(v.get("then_run").ok_or("condition thiếu then_run")?)?;
                Rule::If(c, Box::new(r))
            }
            "bandlands" => Rule::Bands,
            other => return Err(format!("chưa hỗ trợ luật bề mặt '{other}'")),
        })
    }

    /// Bảng 192 dải đất sét màu dùng cho vùng đất hoang (bandlands).
    fn clay_bands(&mut self) -> Vec<u16> {
        let mut r = self.fac.from_hash_of("minecraft:clay_bands");
        let ter = self.intern("terracotta");
        let ora = self.intern("orange_terracotta");
        let yel = self.intern("yellow_terracotta");
        let bro = self.intern("brown_terracotta");
        let red = self.intern("red_terracotta");
        let whi = self.intern("white_terracotta");
        let gry = self.intern("light_gray_terracotta");
        let n = 192usize;
        let mut b = vec![ter; n];
        let mut i = 0usize;
        while i < n {
            i += r.next_bounded(5) as usize + 1;
            if i < n {
                b[i] = ora;
            }
            i += 1;
        }
        let mut make = |r: &mut Xoro, b: &mut Vec<u16>, min: i32, st: u16| {
            let cnt = r.next_between(6, 15);
            for _ in 0..cnt {
                let sz = (min + r.next_bounded(3)) as usize;
                let start = r.next_bounded(n as i32) as usize;
                let mut m = 0usize;
                while start + m < n && m < sz {
                    b[start + m] = st;
                    m += 1;
                }
            }
        };
        make(&mut r, &mut b, 1, yel);
        make(&mut r, &mut b, 2, bro);
        make(&mut r, &mut b, 1, red);
        let count = r.next_between(9, 15);
        let (mut j, mut k) = (0, 0usize);
        while j < count && k < n {
            b[k] = whi;
            if k > 1 && r.next_bool() {
                b[k - 1] = gry;
            }
            if k + 1 < n && r.next_bool() {
                b[k + 1] = gry;
            }
            k += r.next_bounded(16) as usize + 4;
            j += 1;
        }
        b
    }
}

impl Router {
    /// Tên khối theo mã trong kết quả của `surface_column`.
    pub fn block_name(&self, id: u16) -> &str {
        &self.names[id as usize]
    }

    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> &'static str {
        if let Some(b) = self.biome_memo.borrow().get(&(qx, qy, qz)) {
            return *b;
        }
        let b = self.bsrc.find(&self.climate(qx, qy, qz));
        self.biome_memo.borrow_mut().insert((qx, qy, qz), b);
        b
    }

    /// Quần xã tại một khối, có làm mờ biên theo hạt giống như Minecraft.
    pub fn fuzzy_biome(&self, x: i32, y: i32, z: i32) -> &'static str {
        let (i, j, k) = (x - 2, y - 2, z - 2);
        let (l, m, n) = (i >> 2, j >> 2, k >> 2);
        let (d, e, f) = ((i & 3) as f64 / 4.0, (j & 3) as f64 / 4.0, (k & 3) as f64 / 4.0);
        let mut best = 0usize;
        let mut g = f64::INFINITY;
        for p in 0..8usize {
            let (bx, by, bz) = (p & 4 == 0, p & 2 == 0, p & 1 == 0);
            let q = if bx { l } else { l + 1 };
            let r = if by { m } else { m + 1 };
            let s = if bz { n } else { n + 1 };
            let h = if bx { d } else { d - 1.0 };
            let t = if by { e } else { e - 1.0 };
            let u = if bz { f } else { f - 1.0 };
            let v = self.fiddled(q, r, s, h, t, u);
            if g > v {
                best = p;
                g = v;
            }
        }
        let w = if best & 4 == 0 { l } else { l + 1 };
        let xx = if best & 2 == 0 { m } else { m + 1 };
        let yy = if best & 1 == 0 { n } else { n + 1 };
        self.noise_biome(w, xx, yy)
    }

    fn fiddled(&self, x: i32, y: i32, z: i32, xn: f64, yn: f64, zn: f64) -> f64 {
        let lcg = |seed: i64, salt: i64| seed.wrapping_mul(seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)).wrapping_add(salt);
        let fid = |l: i64| (((l >> 24) & 1023) as f64 / 1024.0 - 0.5) * 0.9;
        let s = self.zoom;
        let mut l = lcg(s, x as i64);
        l = lcg(l, y as i64);
        l = lcg(l, z as i64);
        l = lcg(l, x as i64);
        l = lcg(l, y as i64);
        l = lcg(l, z as i64);
        let a = fid(l);
        l = lcg(l, s);
        let b = fid(l);
        l = lcg(l, s);
        let c = fid(l);
        let sq = |v: f64| v * v;
        sq(zn + c) + sq(yn + b) + sq(xn + a)
    }

    fn prelim_at(&self, x: i32, z: i32) -> i32 {
        if let Some(v) = self.prelim_memo.borrow().get(&(x, z)) {
            return *v;
        }
        let v = self.eval(&self.prelim, &Ctx { x, y: 0, z, corner: true }).floor() as i32;
        self.prelim_memo.borrow_mut().insert((x, z), v);
        v
    }

    fn min_surface_level(&self, x: i32, z: i32) -> i32 {
        let (cx, cz) = (x >> 4, z >> 4);
        let g = |a: i32, b: i32| self.prelim_at(a * 16, b * 16) as f64;
        let (v00, v10, v01, v11) = (g(cx, cz), g(cx + 1, cz), g(cx, cz + 1), g(cx + 1, cz + 1));
        let fx = ((x & 15) as f32 / 16.0) as f64;
        let fz = ((z & 15) as f32 / 16.0) as f64;
        lerp(fz, lerp(fx, v00, v10), lerp(fx, v01, v11)).floor() as i32
    }

    fn band(&self, x: i32, y: i32, z: i32) -> u16 {
        let off = (self.noises[self.band_noise].value(x as f64, 0.0, z as f64) * 4.0 + 0.5).floor() as i32;
        let n = self.bands.len() as i32;
        self.bands[((y + off + n) % n) as usize]
    }

    fn cond(&self, c: &Cond, s: &SCtx) -> bool {
        match c {
            Cond::Biome(l) => {
                let b = self.fuzzy_biome(s.x, s.y, s.z);
                l.iter().any(|n| n == b)
            }
            Cond::Noise { n, lo, hi } => {
                let v = self.noises[*n].value(s.x as f64, 0.0, s.z as f64);
                v >= *lo && v <= *hi
            }
            Cond::VGrad { fac, t, f } => {
                let (t, f) = (t.y(self.min_y, self.height), f.y(self.min_y, self.height));
                if s.y <= t {
                    true
                } else if s.y >= f {
                    false
                } else {
                    let m = 1.0 - (s.y - t) as f64 / (f - t) as f64;
                    (fac.at(s.x, s.y, s.z).next_f32() as f64) < m
                }
            }
            Cond::YAbove { a, mult, add } => (s.y + (if *add { s.stone_above } else { 0 })) >= a.y(self.min_y, self.height) + s.run_depth * *mult,
            Cond::Water { offset, mult, add } => s.fluid_h == i32::MIN || (s.y + (if *add { s.stone_above } else { 0 })) >= s.fluid_h + *offset + s.run_depth * *mult,
            Cond::Temp => {
                let b = self.fuzzy_biome(s.x, s.y, s.z);
                self.temps.get(b).copied().unwrap_or(0.5) < 0.15
            }
            Cond::Steep => s.steep,
            Cond::Hole => s.run_depth <= 0,
            Cond::Prelim => s.y >= s.min_surface,
            Cond::Not(c) => !self.cond(c, s),
            Cond::Depth { offset, add, range, ceiling } => {
                let d = if *ceiling { s.stone_below } else { s.stone_above };
                let sec = if *range == 0 { 0 } else { ((s.secondary + 1.0) / 2.0 * *range as f64) as i32 };
                d <= 1 + *offset + (if *add { s.run_depth } else { 0 }) + sec
            }
        }
    }

    fn apply(&self, r: &Rule, s: &SCtx) -> Option<u16> {
        match r {
            Rule::Block(id) => Some(*id),
            Rule::Seq(v) => {
                for x in v {
                    if let Some(b) = self.apply(x, s) {
                        return Some(b);
                    }
                }
                None
            }
            Rule::If(c, t) => {
                if self.cond(c, s) {
                    self.apply(t, s)
                } else {
                    None
                }
            }
            Rule::Bands => Some(self.band(s.x, s.y, s.z)),
        }
    }

    /// Áp luật bề mặt cho một cột. `solid[i]` cho khối ở độ cao min_y + i. Trả về mã khối từng độ cao (0 không khí, 1 nước, 2 đá, còn lại tra `block_name`).
    pub fn surface_column(&self, x: i32, z: i32, solid: &[bool], steep: bool) -> Vec<u16> {
        let h = solid.len();
        let min = self.min_y;
        let mut b: Vec<u16> = (0..h).map(|i| if solid[i] { 2 } else if min + i as i32 <= self.sea { 1 } else { 0 }).collect();
        let mut top = 0usize;
        for i in (0..h).rev() {
            if b[i] != 0 {
                top = i + 1;
                break;
            }
        }
        let sn = self.noises[self.surf_noise].value(x as f64, 0.0, z as f64);
        let mut rnd = self.fac.at(x, 0, z);
        let run_depth = (sn * 2.75 + 3.0 + rnd.next_f64() * 0.25) as i32;
        let secondary = self.noises[self.sec_noise].value(x as f64, 0.0, z as f64);
        let min_surface = self.min_surface_level(x, z) + run_depth - 8;
        let (mut above, mut fluid_h, mut bottom, mut prev_solid) = (0i32, i32::MIN, 0usize, false);
        for i in (0..top).rev() {
            let y = min + i as i32;
            match b[i] {
                0 => {
                    above = 0;
                    fluid_h = i32::MIN;
                    prev_solid = false;
                }
                1 => {
                    if fluid_h == i32::MIN {
                        fluid_h = y + 1;
                    }
                    prev_solid = false;
                }
                _ => {
                    if !prev_solid {
                        let mut j = i;
                        while j > 0 && b[j - 1] >= 2 {
                            j -= 1;
                        }
                        bottom = j;
                    }
                    prev_solid = true;
                    above += 1;
                    let s = SCtx { x, y, z, run_depth, secondary, stone_above: above, stone_below: (i - bottom + 1) as i32, fluid_h, min_surface, steep };
                    if let Some(id) = self.apply(&self.rule, &s) {
                        b[i] = id;
                    }
                }
            }
        }
        b
    }
}

/// Điều kiện `steep`: so độ cao mặt đất của ô kề trong cùng chunk.
pub fn steep_at(top: &dyn Fn(i32, i32) -> i32, lx: i32, lz: i32) -> bool {
    let (k, l) = ((lz - 1).max(0), (lz + 1).min(15));
    if top(lx, l) >= top(lx, k) + 4 {
        return true;
    }
    let (o, p) = ((lx - 1).max(0), (lx + 1).min(15));
    top(p, lz) >= top(o, lz) + 4
}
