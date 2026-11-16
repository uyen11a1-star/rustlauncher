//! Đọc tài nguyên từ client.jar của Minecraft: atlas texture khối + model khối (JSON, kế thừa parent) -> đỉnh để vẽ.
use std::{collections::HashMap, path::Path};

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
        let stem = &n[prefix.len()..n.len() - 4];
        tiles.insert(stem.to_string(), (tx, ty));
        slot += 1;
    }
    log(format!("Atlas: {} texture, bỏ qua {} file.", tiles.len(), bad));
    Ok(Atlas { pixels, tiles })
}

// ---------- model khối ----------

#[derive(Clone)]
pub struct Face {
    pub dir: usize, // 0 down, 1 up, 2 north, 3 south, 4 west, 5 east
    pub uv: Option<[f32; 4]>,
    pub tex: String,
    pub tint: bool,
    pub rot: i32,
}

#[derive(Clone)]
pub struct Element {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub shade: bool,
    pub faces: Vec<Face>,
}

#[derive(Clone, Default)]
pub struct Model {
    pub elements: Vec<Element>,
    pub textures: HashMap<String, String>,
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

fn parse_elements(arr: &[Value]) -> Vec<Element> {
    let dirs = ["down", "up", "north", "south", "west", "east"];
    let mut out = Vec::new();
    for el in arr {
        let (Some(from), Some(to)) = (vec3(el.get("from")), vec3(el.get("to"))) else { continue };
        let mut faces = Vec::new();
        if let Some(Value::Obj(fm)) = el.get("faces") {
            for (i, d) in dirs.iter().enumerate() {
                let Some(f) = fm.get(*d) else { continue };
                let Some(tex) = f.get("texture").and_then(|t| t.as_str()) else { continue };
                let uv = f.get("uv").and_then(|u| u.as_arr()).and_then(|u| {
                    if u.len() == 4 {
                        Some([u[0].as_f64()? as f32, u[1].as_f64()? as f32, u[2].as_f64()? as f32, u[3].as_f64()? as f32])
                    } else {
                        None
                    }
                });
                faces.push(Face {
                    dir: i,
                    uv,
                    tex: tex.to_string(),
                    tint: f.get("tintindex").is_some(),
                    rot: f.get("rotation").and_then(|r| r.as_f64()).unwrap_or(0.0) as i32,
                });
            }
        }
        out.push(Element { from, to, shade: el.get("shade").and_then(|s| s.as_bool()).unwrap_or(true), faces });
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
        Some(p) if !p.starts_with("builtin/") => load_model(jar, p, depth + 1)?,
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

const SHADE: [f32; 6] = [0.5, 1.0, 0.8, 0.8, 0.6, 0.6];
const TINT: [f32; 3] = [121.0, 192.0, 90.0];

/// Sinh đỉnh cho mọi mặt của model, đặt tại `at` (đơn vị khối).
pub fn emit(m: &Model, atlas: &Atlas, at: [f32; 3], out: &mut Vec<V>) {
    for el in &m.elements {
        let (f, t) = (el.from, el.to);
        let (x0, y0, z0) = (f[0] / 16.0, f[1] / 16.0, f[2] / 16.0);
        let (x1, y1, z1) = (t[0] / 16.0, t[1] / 16.0, t[2] / 16.0);
        for face in &el.faces {
            let Some(name) = resolve_tex(&face.tex, &m.textures) else { continue };
            let Some(&(tx, ty)) = atlas.tiles.get(&name) else { continue };
            // 4 đỉnh: trái-trên, trái-dưới, phải-dưới, phải-trên (nhìn từ ngoài vào)
            let q: [[f32; 3]; 4] = match face.dir {
                0 => [[x0, y0, z1], [x0, y0, z0], [x1, y0, z0], [x1, y0, z1]],
                1 => [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
                2 => [[x1, y1, z0], [x1, y0, z0], [x0, y0, z0], [x0, y1, z0]],
                3 => [[x0, y1, z1], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1]],
                4 => [[x0, y1, z0], [x0, y0, z0], [x0, y0, z1], [x0, y1, z1]],
                _ => [[x1, y1, z1], [x1, y0, z1], [x1, y0, z0], [x1, y1, z0]],
            };
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
            let shade = if el.shade { SHADE[face.dir] } else { 1.0 };
            let base = if face.tint { TINT } else { [255.0, 255.0, 255.0] };
            let c = [(base[0] * shade) as u8, (base[1] * shade) as u8, (base[2] * shade) as u8, 255];
            let a = ATLAS as f32;
            let vert = |i: usize| V {
                p: [q[i][0] + at[0], q[i][1] + at[1], q[i][2] + at[2]],
                uv: [((tx * TILE) as f32 + corners[i][0]) / a, ((ty * TILE) as f32 + corners[i][1]) / a],
                c,
            };
            for i in [0usize, 1, 2, 0, 2, 3] {
                out.push(vert(i));
            }
        }
    }
}
