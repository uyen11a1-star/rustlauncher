//! Từ block state ("minecraft:oak_stairs[facing=east,half=bottom]") ra danh sách mặt để vẽ,
//! dựa trên blockstates/*.json và models/block/*.json THẬT trong client.jar.
use std::{collections::HashMap, sync::Arc};

use crate::{
    assets::{bake, load_model, occlusion_mask, Atlas, BFace, Jar, Model},
    json::{self, Value},
};

pub struct State {
    pub air: bool,
    /// 0 không phải chất lỏng, 1 nước, 2 dung nham
    pub fluid: u8,
    /// bit d bật: mặt hướng d của khối này che kín khối kề
    pub occl: u8,
    pub tint: [u8; 3],
    pub faces: Vec<BFace>,
}

pub struct Resolver<'a> {
    jar: &'a Jar,
    atlas: &'a Atlas,
    bs: HashMap<String, Option<Value>>,
    models: HashMap<String, Option<Arc<Model>>>,
    baked: HashMap<(String, i32, i32), Arc<Vec<BFace>>>,
    pub missing: Vec<String>,
}

fn split_key(key: &str) -> (String, Vec<(String, String)>) {
    let key = key.strip_prefix("minecraft:").unwrap_or(key);
    match key.find('[') {
        Some(i) => {
            let props = key[i + 1..].trim_end_matches(']');
            let v = props
                .split(',')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            (key[..i].to_string(), v)
        }
        None => (key.to_string(), Vec::new()),
    }
}

fn tint_for(name: &str) -> [u8; 3] {
    if name.contains("birch_leaves") {
        [128, 167, 85]
    } else if name.contains("spruce_leaves") {
        [97, 153, 97]
    } else if name.ends_with("leaves") {
        [72, 181, 24]
    } else if name == "water" {
        [63, 118, 228]
    } else if name == "lily_pad" {
        [32, 128, 48]
    } else {
        [121, 192, 90]
    }
}

fn variant_matches(key: &str, props: &[(String, String)]) -> bool {
    if key.is_empty() || key == "normal" {
        return true;
    }
    key.split(',').all(|kv| match kv.split_once('=') {
        Some((k, v)) => props.iter().any(|(pk, pv)| pk == k && pv == v),
        None => false,
    })
}

fn when_ok(w: &Value, props: &[(String, String)]) -> bool {
    if let Some(arr) = w.get("OR").and_then(|a| a.as_arr()) {
        return arr.iter().any(|c| when_ok(c, props));
    }
    if let Some(arr) = w.get("AND").and_then(|a| a.as_arr()) {
        return arr.iter().all(|c| when_ok(c, props));
    }
    if let Value::Obj(m) = w {
        return m.iter().all(|(k, v)| {
            let want = match v {
                Value::Str(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                _ => return false,
            };
            let have = props.iter().find(|(pk, _)| pk == k).map(|(_, pv)| pv.as_str()).unwrap_or("");
            want.split('|').any(|x| x == have)
        });
    }
    true
}

fn pick(v: &Value, out: &mut Vec<(String, i32, i32)>) {
    let v = match v.as_arr() {
        Some(a) => match a.first() {
            Some(x) => x,
            None => return,
        },
        None => v,
    };
    if let Some(m) = v.get("model").and_then(|m| m.as_str()) {
        let x = v.get("x").and_then(|x| x.as_f64()).unwrap_or(0.0) as i32;
        let y = v.get("y").and_then(|y| y.as_f64()).unwrap_or(0.0) as i32;
        out.push((m.to_string(), x, y));
    }
}

fn select(v: &Value, props: &[(String, String)]) -> Vec<(String, i32, i32)> {
    let mut out = Vec::new();
    if let Some(Value::Obj(vars)) = v.get("variants") {
        let mut first = None;
        for (k, val) in vars {
            if first.is_none() {
                first = Some(val);
            }
            if variant_matches(k, props) {
                pick(val, &mut out);
                return out;
            }
        }
        if let Some(val) = first {
            pick(val, &mut out);
        }
    } else if let Some(parts) = v.get("multipart").and_then(|m| m.as_arr()) {
        for part in parts {
            if part.get("when").map_or(true, |w| when_ok(w, props)) {
                if let Some(a) = part.get("apply") {
                    pick(a, &mut out);
                }
            }
        }
    }
    out
}

impl<'a> Resolver<'a> {
    pub fn new(jar: &'a Jar, atlas: &'a Atlas) -> Self {
        Resolver { jar, atlas, bs: HashMap::new(), models: HashMap::new(), baked: HashMap::new(), missing: Vec::new() }
    }

    fn blockstate(&mut self, name: &str) -> Option<&Value> {
        if !self.bs.contains_key(name) {
            let path = format!("assets/minecraft/blockstates/{name}.json");
            let v = self.jar.read(&path).ok().and_then(|b| String::from_utf8(b).ok()).and_then(|t| json::parse(&t).ok());
            self.bs.insert(name.to_string(), v);
        }
        self.bs.get(name).and_then(|v| v.as_ref())
    }

    fn model(&mut self, name: &str) -> Option<Arc<Model>> {
        if let Some(m) = self.models.get(name) {
            return m.clone();
        }
        let m = match load_model(self.jar, name, 0) {
            Ok(m) => Some(Arc::new(m)),
            Err(e) => {
                self.missing.push(format!("model {name}: {e}"));
                None
            }
        };
        self.models.insert(name.to_string(), m.clone());
        m
    }

    fn baked(&mut self, name: &str, rx: i32, ry: i32) -> Option<Arc<Vec<BFace>>> {
        let key = (name.to_string(), rx, ry);
        if let Some(b) = self.baked.get(&key) {
            return Some(b.clone());
        }
        let m = self.model(name)?;
        let b = Arc::new(bake(&m, self.atlas, rx, ry));
        self.baked.insert(key, b.clone());
        Some(b)
    }

    pub fn state(&mut self, key: &str) -> State {
        let (name, props) = split_key(key);
        let air = |_: ()| State { air: true, fluid: 0, occl: 0, tint: [255; 3], faces: Vec::new() };
        if matches!(name.as_str(), "air" | "cave_air" | "void_air" | "light" | "barrier" | "structure_void" | "bubble_column") {
            return air(());
        }
        let tint = tint_for(&name);
        let (faces, fluid): (Vec<BFace>, u8) = if name == "water" || name == "lava" {
            let tex = if name == "water" { "water_still" } else { "lava_still" };
            let b = bake(&Model::cube(tex), self.atlas, 0, 0);
            let b = if name == "lava" { b.into_iter().map(|mut f| { f.tint = false; f }).collect() } else { b };
            (b, if name == "water" { 1 } else { 2 })
        } else {
            let sel = match self.blockstate(&name) {
                Some(v) => select(v, &props),
                None => {
                    self.missing.push(format!("blockstate {name}"));
                    Vec::new()
                }
            };
            let mut faces = Vec::new();
            for (m, rx, ry) in sel {
                if let Some(b) = self.baked(&m, rx, ry) {
                    faces.extend(b.iter().cloned());
                }
            }
            (faces, 0)
        };
        let occl = if fluid == 0 { occlusion_mask(&faces, self.atlas) } else { 0 };
        State { air: false, fluid, occl, tint, faces }
    }
}
