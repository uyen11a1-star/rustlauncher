//! Đọc tài nguyên từ client.jar của Minecraft: atlas texture khối + model khối (JSON, kế thừa parent) -> đỉnh để vẽ.
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    json::{self, Value},
    png, zipr,
};

pub const ATLAS: usize = 1024;
pub const TILE: usize = 16;
const PER_ROW: usize = ATLAS / TILE;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct V {
    pub p: [f32; 3],
    pub uv: [f32; 2],
    pub c: [u8; 4],
}

pub struct Jar {
    bytes: Vec<u8>,
    pub entries: HashMap<String, zipr::Entry>,
}

impl Jar {
    pub fn open(path: &Path) -> Result<Jar, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("không đọc được {}: {e}", path.display()))?;
        let tail_start = bytes.len().saturating_sub(262_144);
        let (off, size) = zipr::find_cd(&bytes[tail_start..]).ok_or("jar: không thấy EOCD")?;
        let end = (off + size) as usize;
        if end > bytes.len() {
            return Err("jar: central directory vượt file".into());
        }
        let list = zipr::parse_central(&bytes[off as usize..end])?;
        Ok(Jar { entries: list.into_iter().map(|e| (e.name.clone(), e)).collect(), bytes })
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let e = self.entries.get(name).ok_or_else(|| format!("jar không có {name}"))?;
        zipr::entry_data(&self.bytes[e.local_off as usize..], e)
    }
}

pub struct Atlas {
    pub pixels: Vec<u8>,
    /// tên texture (không đuôi) -> ô (cột, hàng)
    pub tiles: HashMap<String, (usize, usize)>,
    /// ô texture không có điểm ảnh trong suốt nào (che được mặt khối kề bên)
    pub opaque: HashSet<(usize, usize)>,
}

pub fn build_atlas(jar: &Jar, log: &dyn Fn(String)) -> Result<Atlas, String> {
    let prefix = "assets/minecraft/textures/block/";
    let mut names: Vec<&String> = jar
        .entries
        .keys()
        .filter(|n| n.starts_with(prefix) && n.ends_with(".png") && !n[prefix.len()..].contains('/'))
        .collect();
    names.sort();
    let mut pixels = vec![0u8; ATLAS * ATLAS * 4];
    let mut tiles = HashMap::new();
    let mut opaque = HashSet::new();
    let mut bad = 0;
    let mut slot = 0usize;
    for n in names {
        if slot >= PER_ROW * PER_ROW {
            break;
        }
        let img = match jar.read(n).and_then(|d| png::decode(&d)) {
            Ok(i) if i.w >= TILE && i.h >= TILE => i,
            _ => {
                bad += 1;
                continue;
            }
        };
        let (tx, ty) = (slot % PER_ROW, slot / PER_ROW);
        // Texture động là dải dọc nhiều khung: lấy khung đầu (góc trên-trái TILE x TILE).
        for y in 0..TILE {
            let dst = ((ty * TILE + y) * ATLAS + tx * TILE) * 4;
            let src = y * img.w * 4;
            pixels[dst..dst + TILE * 4].copy_from_slice(&img.rgba[src..src + TILE * 4]);
        }
        if (0..TILE * TILE).all(|i| {
            let (x, y) = (i % TILE, i / TILE);
            img.rgba[(y * img.w + x) * 4 + 3] == 255
        }) {
            opaque.insert((tx, ty));
        }
        let stem = &n[prefix.len()..n.len() - 4];
        tiles.insert(stem.to_string(), (tx, ty));
        slot += 1;
    }
    log(format!("Atlas: {} texture, bỏ qua {} file.", tiles.len(), bad));
    Ok(Atlas { pixels, tiles, opaque })
}

// ---------- model khối ----------

#[derive(Clone)]
pub struct Face {
    pub dir: usize, // 0 down, 1 up, 2 north, 3 south, 4 west, 5 east
    pub uv: Option<[f32; 4]>,
    pub tex: String,
    pub tint: bool,
    pub rot: i32,
    pub cull: Option<usize>,
}

#[derive(Clone)]
pub struct ElRot {
    pub axis: u8, // 0 x, 1 y, 2 z
    pub origin: [f32; 3],
    pub angle: f32,
    pub rescale: bool,
}

#[derive(Clone)]
pub struct Element {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub shade: bool,
    pub rot: Option<ElRot>,
    pub faces: Vec<Face>,
}

#[derive(Clone, Default)]
pub struct Model {
    pub elements: Vec<Element>,
    pub textures: HashMap<String, String>,
}

impl Model {
    /// Khối đặc 16x16x16 dùng một texture cho cả 6 mặt (dùng cho nước, dung nham).
    pub fn cube(tex: &str) -> Model {
        let faces = (0..6)
            .map(|d| Face { dir: d, uv: None, tex: "#all".into(), tint: true, rot: 0, cull: Some(d) })
            .collect();
        let mut textures = HashMap::new();
        textures.insert("all".to_string(), format!("block/{tex}"));
        Model { elements: vec![Element { from: [0.0; 3], to: [16.0; 3], shade: true, rot: None, faces }], textures }
    }
}

fn strip_ns(s: &str) -> &str {
    s.strip_prefix("minecraft:").unwrap_or(s)
}

fn vec3(v: Option<&Value>) -> Option<[f32; 3]> {
    let a = v?.as_arr()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32])
}

const DIRS: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

fn parse_elements(arr: &[Value]) -> Vec<Element> {
    let mut out = Vec::new();
    for el in arr {
        let (Some(from), Some(to)) = (vec3(el.get("from")), vec3(el.get("to"))) else { continue };
        let mut faces = Vec::new();
        if let Some(Value::Obj(fm)) = el.get("faces") {
            for (i, d) in DIRS.iter().enumerate() {
                let Some(f) = fm.get(*d) else { continue };
                let Some(tex) = f.get("texture").and_then(|t| t.as_str()) else { continue };
                let uv = f.get("uv").and_then(|u| u.as_arr()).and_then(|u| {
                    if u.len() == 4 {
                        Some([u[0].as_f64()? as f32, u[1].as_f64()? as f32, u[2].as_f64()? as f32, u[3].as_f64()? as f32])
                    } else {
                        None
                    }
                });
                let cull = f.get("cullface").and_then(|c| c.as_str()).and_then(|c| DIRS.iter().position(|d| *d == c));
                faces.push(Face {
                    dir: i,
                    uv,
                    tex: tex.to_string(),
                    tint: f.get("tintindex").is_some(),
                    rot: f.get("rotation").and_then(|r| r.as_f64()).unwrap_or(0.0) as i32,
                    cull,
                });
            }
        }
        let rot = el.get("rotation").and_then(|r| {
            let axis = match r.get("axis")?.as_str()? {
                "x" => 0,
                "y" => 1,
                _ => 2,
            };
            Some(ElRot {
                axis,
                origin: vec3(r.get("origin")).unwrap_or([8.0, 8.0, 8.0]),
                angle: r.get("angle")?.as_f64()? as f32,
                rescale: r.get("rescale").and_then(|b| b.as_bool()).unwrap_or(false),
            })
        });
        out.push(Element { from, to, shade: el.get("shade").and_then(|s| s.as_bool()).unwrap_or(true), rot, faces });
    }
    out
}

/// Nạp model `name` (vd "block/grass_block"), gộp textures và elements theo chuỗi parent.
pub fn load_model(jar: &Jar, name: &str, depth: u32) -> Result<Model, String> {
    if depth > 16 {
        return Err("chuỗi parent quá sâu".into());
    }
    let path = format!("assets/minecraft/models/{}.json", strip_ns(name));
    let text = String::from_utf8(jar.read(&path)?).map_err(|_| "model không phải UTF-8".to_string())?;
    let v = json::parse(&text)?;
    let mut m = match v.get("parent").and_then(|p| p.as_str()) {
        Some(p) if !p.contains("builtin/") => load_model(jar, p, depth + 1)?,
        _ => Model::default(),
    };
    if let Some(Value::Obj(t)) = v.get("textures") {
        for (k, val) in t {
            if let Some(s) = val.as_str() {
                m.textures.insert(k.clone(), s.to_string());
            }
        }
    }
    if let Some(arr) = v.get("elements").and_then(|e| e.as_arr()) {
        m.elements = parse_elements(arr);
    }
    Ok(m)
}

/// Giải biến `#xxx` rồi bỏ tiền tố, trả về tên file texture khối (không đuôi).
fn resolve_tex(t: &str, vars: &HashMap<String, String>) -> Option<String> {
    let mut cur = t.to_string();
    for _ in 0..16 {
        if let Some(k) = cur.strip_prefix('#') {
            cur = vars.get(k)?.clone();
        } else {
            let s = strip_ns(&cur);
            return s.strip_prefix("block/").map(|x| x.to_string());
        }
    }
    None
}

// ---------- mặt đã "nướng" sẵn ----------

pub const NOCULL: u8 = 255;

/// Một mặt đã có tọa độ cuối cùng trong khối 1x1x1 (đã áp xoay phần tử và xoay khối), UV chuẩn hóa theo atlas.
#[derive(Clone)]
pub struct BFace {
    pub q: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub cull: u8,
    pub shade: f32,
    pub tint: bool,
}

const SHADE: [f32; 6] = [0.5, 1.0, 0.8, 0.8, 0.6, 0.6];
const DV: [[f32; 3]; 6] = [[0.0, -1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]];

/// Xoay vector (tương đối tâm) `kx` bước 90° quanh X rồi `ky` bước quanh Y, theo quy ước của Minecraft.
fn rot_vec(mut v: [f32; 3], kx: i32, ky: i32) -> [f32; 3] {
    for _ in 0..kx {
        v = [v[0], v[2], -v[1]];
    }
    for _ in 0..ky {
        v = [-v[2], v[1], v[0]];
    }
    v
}

fn vec_dir(v: [f32; 3]) -> usize {
    let (ax, ay, az) = (v[0].abs(), v[1].abs(), v[2].abs());
    if ay >= ax && ay >= az {
        if v[1] >= 0.0 { 1 } else { 0 }
    } else if az >= ax {
        if v[2] >= 0.0 { 3 } else { 2 }
    } else if v[0] >= 0.0 {
        5
    } else {
        4
    }
}

fn elem_rotate(p: [f32; 3], r: &ElRot) -> [f32; 3] {
    let o = [r.origin[0] / 16.0, r.origin[1] / 16.0, r.origin[2] / 16.0];
    let (s, c) = r.angle.to_radians().sin_cos();
    let d = [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
    let mut n = match r.axis {
        0 => [d[0], d[1] * c - d[2] * s, d[1] * s + d[2] * c],
        1 => [d[0] * c + d[2] * s, d[1], -d[0] * s + d[2] * c],
        _ => [d[0] * c - d[1] * s, d[0] * s + d[1] * c, d[2]],
    };
    if r.rescale && c.abs() > 1e-4 {
        let k = 1.0 / c.abs();
        match r.axis {
            0 => {
                n[1] *= k;
                n[2] *= k;
            }
            1 => {
                n[0] *= k;
                n[2] *= k;
            }
            _ => {
                n[0] *= k;
                n[1] *= k;
            }
        }
    }
    [n[0] + o[0], n[1] + o[1], n[2] + o[2]]
}

/// Biến model thành danh sách mặt, xoay khối `rx` độ quanh X rồi `ry` độ quanh Y (bội của 90).
pub fn bake(m: &Model, atlas: &Atlas, rx: i32, ry: i32) -> Vec<BFace> {
    let (kx, ky) = (rx.rem_euclid(360) / 90, ry.rem_euclid(360) / 90);
    let a = ATLAS as f32;
    let mut out = Vec::new();
    for el in &m.elements {
        let (f, t) = (el.from, el.to);
        let (x0, y0, z0) = (f[0] / 16.0, f[1] / 16.0, f[2] / 16.0);
        let (x1, y1, z1) = (t[0] / 16.0, t[1] / 16.0, t[2] / 16.0);
        for face in &el.faces {
            let Some(name) = resolve_tex(&face.tex, &m.textures) else { continue };
            let Some(&(tx, ty)) = atlas.tiles.get(&name) else { continue };
            // 4 đỉnh: trái-trên, trái-dưới, phải-dưới, phải-trên (nhìn từ ngoài vào)
            let mut q: [[f32; 3]; 4] = match face.dir {
                0 => [[x0, y0, z1], [x0, y0, z0], [x1, y0, z0], [x1, y0, z1]],
                1 => [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
                2 => [[x1, y1, z0], [x1, y0, z0], [x0, y0, z0], [x0, y1, z0]],
                3 => [[x0, y1, z1], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1]],
                4 => [[x0, y1, z0], [x0, y0, z0], [x0, y0, z1], [x0, y1, z1]],
                _ => [[x1, y1, z1], [x1, y0, z1], [x1, y0, z0], [x1, y1, z0]],
            };
            if let Some(r) = &el.rot {
                for p in q.iter_mut() {
                    *p = elem_rotate(*p, r);
                }
            }
            if kx != 0 || ky != 0 {
                for p in q.iter_mut() {
                    let v = rot_vec([p[0] - 0.5, p[1] - 0.5, p[2] - 0.5], kx, ky);
                    *p = [v[0] + 0.5, v[1] + 0.5, v[2] + 0.5];
                }
            }
            let uv = face.uv.unwrap_or(match face.dir {
                0 => [f[0], 16.0 - t[2], t[0], 16.0 - f[2]],
                1 => [f[0], f[2], t[0], t[2]],
                2 => [16.0 - t[0], 16.0 - t[1], 16.0 - f[0], 16.0 - f[1]],
                3 => [f[0], 16.0 - t[1], t[0], 16.0 - f[1]],
                4 => [f[2], 16.0 - t[1], t[2], 16.0 - f[1]],
                _ => [16.0 - t[2], 16.0 - t[1], 16.0 - f[2], 16.0 - f[1]],
            });
            let mut corners = [[uv[0], uv[1]], [uv[0], uv[3]], [uv[2], uv[3]], [uv[2], uv[1]]];
            for _ in 0..(face.rot.rem_euclid(360) / 90) {
                corners = [corners[1], corners[2], corners[3], corners[0]];
            }
            let mut uvn = [[0.0f32; 2]; 4];
            for i in 0..4 {
                uvn[i] = [((tx * TILE) as f32 + corners[i][0]) / a, ((ty * TILE) as f32 + corners[i][1]) / a];
            }
            // hướng thật của mặt (để tô bóng): pháp tuyến từ hình học cuối cùng
            let e1 = [q[1][0] - q[0][0], q[1][1] - q[0][1], q[1][2] - q[0][2]];
            let e2 = [q[3][0] - q[0][0], q[3][1] - q[0][1], q[3][2] - q[0][2]];
            let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let shade = if el.shade { SHADE[vec_dir(n)] } else { 1.0 };
            let cull = match face.cull {
                Some(c) => vec_dir(rot_vec(DV[c], kx, ky)) as u8,
                None => NOCULL,
            };
            out.push(BFace { q, uv: uvn, cull, shade, tint: face.tint });
        }
    }
    out
}

/// Bit `d` bật nếu mặt hướng `d` của khối này phủ kín cả mặt 1x1 và texture đặc (che được khối kề).
pub fn occlusion_mask(faces: &[BFace], atlas: &Atlas) -> u8 {
    let mut mask = 0u8;
    let a = ATLAS as f32;
    for f in faces {
        if f.cull == NOCULL {
            continue;
        }
        let d = f.cull as usize;
        let ax = d / 2; // 0: y (down/up), 1: z (north/south), 2: x (west/east) -> đổi sang trục thật
        let axis = [1usize, 2, 0][ax];
        let plane = if d % 2 == 0 { 0.0 } else { 1.0 };
        if f.q.iter().any(|p| (p[axis] - plane).abs() > 1e-4) {
            continue;
        }
        let others = [0usize, 1, 2].into_iter().filter(|&i| i != axis).collect::<Vec<_>>();
        let full = others.iter().all(|&i| {
            let lo = f.q.iter().map(|p| p[i]).fold(f32::MAX, f32::min);
            let hi = f.q.iter().map(|p| p[i]).fold(f32::MIN, f32::max);
            lo < 1e-4 && hi > 1.0 - 1e-4
        });
        if !full {
            continue;
        }
        // dùng điểm giữa 4 góc UV để tìm ô texture (góc có thể nằm đúng mép ô kế tiếp)
        let cu = (f.uv.iter().map(|u| u[0]).sum::<f32>() / 4.0 * a / TILE as f32) as usize;
        let cv = (f.uv.iter().map(|u| u[1]).sum::<f32>() / 4.0 * a / TILE as f32) as usize;
        if atlas.opaque.contains(&(cu, cv)) {
            mask |= 1 << d;
        }
    }
    mask
}

// ---------- lưới tam giác ----------

#[derive(Default)]
pub struct Mesh {
    pub v: Vec<V>,
    pub i: Vec<u32>,
    /// chỉ số của các mặt nước (vẽ riêng, nửa trong suốt)
    pub iw: Vec<u32>,
}

impl Mesh {
    pub fn push_face(&mut self, f: &BFace, at: [f32; 3], col: [u8; 3]) {
        self.push_face_b(f, at, col, 1.0);
    }

    /// Như `push_face` nhưng nhân thêm độ sáng `br` (0..1) vào màu đỉnh.
    pub fn push_face_b(&mut self, f: &BFace, at: [f32; 3], col: [u8; 3], br: f32) {
        let base = self.v.len() as u32;
        let s = f.shade * br;
        let c = [(col[0] as f32 * s) as u8, (col[1] as f32 * s) as u8, (col[2] as f32 * s) as u8, 255];
        for k in 0..4 {
            self.v.push(V { p: [f.q[k][0] + at[0], f.q[k][1] + at[1], f.q[k][2] + at[2]], uv: f.uv[k], c });
        }
        self.i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Như `push_face` nhưng cho mặt nước: alpha 200, chỉ số vào danh sách `iw`.
    pub fn push_water(&mut self, f: &BFace, at: [f32; 3], col: [u8; 3], br: f32) {
        let base = self.v.len() as u32;
        let s = f.shade * br;
        let c = [(col[0] as f32 * s) as u8, (col[1] as f32 * s) as u8, (col[2] as f32 * s) as u8, 200];
        for k in 0..4 {
            self.v.push(V { p: [f.q[k][0] + at[0], f.q[k][1] + at[1], f.q[k][2] + at[2]], uv: f.uv[k], c });
        }
        self.iw.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn append(&mut self, o: Mesh) {
        let off = self.v.len() as u32;
        self.v.extend(o.v);
        self.i.extend(o.i.into_iter().map(|x| x + off));
        self.iw.extend(o.iw.into_iter().map(|x| x + off));
    }
}
