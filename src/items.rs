//! Vật phẩm cầm tay và ô đồ: mesh khối cho icon GUI / khối cầm tay, tay Steve góc nhìn thứ nhất, ma trận hoạt ảnh như Minecraft.
use std::{collections::HashMap, f32::consts::PI};

use crate::{
    assets::{bake, load_model, Atlas, BFace, Jar, Mesh, V},
    craft::Recipe,
    blocks::Resolver,
    json::{self, Value},
    math::{self, Mat4},
    png,
};

/// Các khối đưa vào túi đồ (bỏ qua khối nào không dựng được). 36 khối đầu có sẵn trong túi, phần còn lại có qua chế tạo.
const KEYS: &[&str] = &[
    "stone", "dirt", "grass_block", "cobblestone", "oak_planks", "oak_log", "sand", "glass", "oak_leaves", "gravel", "bricks", "bookshelf", "crafting_table", "furnace", "tnt", "stone_bricks", "sandstone", "obsidian", "glowstone",
    "white_wool", "red_wool", "birch_planks", "spruce_planks", "birch_log", "spruce_log", "gold_block", "iron_block", "diamond_block", "coal_ore", "iron_ore", "diorite", "granite", "andesite", "carved_pumpkin", "snow_block", "clay",
    "netherrack", "pumpkin", "melon", "bedrock", "polished_andesite", "polished_diorite", "polished_granite", "cut_sandstone",
];

pub struct Item {
    pub name: String,
    /// (chỉ số đầu, số chỉ số) trong `Items::i`: bản vẽ icon GUI / bản cầm tay
    pub gui: (i32, i32),
    pub hand: (i32, i32),
    /// hình phẳng (vật phẩm kiểu sprite): vẽ không xoay 3D
    pub flat: bool,
}

pub struct Skin {
    pub rgba: Vec<u8>,
    pub w: i32,
    pub h: i32,
}

pub struct Tex {
    pub rgba: Vec<u8>,
    pub w: i32,
    pub h: i32,
}

/// Một hình nhỏ trong ảnh HUD: chỉ số ảnh + vùng [x,y,w,h].
#[derive(Clone, Copy)]
pub struct Spr {
    pub tex: usize,
    pub r: [f32; 4],
}

pub struct Hud {
    pub heart_bg: Spr,
    pub heart_full: Spr,
    pub heart_half: Spr,
    pub food_bg: Spr,
    pub food_full: Spr,
    pub food_half: Spr,
    pub xp_bg: Spr,
}

pub struct Items {
    pub hud: Option<Hud>,
    pub hud_tex: Vec<Tex>,
    /// số chỉ số của phần đầu (đầu + mũ) ở đầu `body_i`
    pub head_n: i32,
    pub wear: Vec<i8>,
    pub recipes: Vec<Recipe>,
    /// ảnh nền túi đồ (256x256, vùng 176x166)
    pub inv: Option<Tex>,
    /// ảnh nền rương/menu server (generic_54.png)
    pub chest: Option<Tex>,
    /// ảnh thanh nhanh + vùng nguồn [x,y,w,h] trong ảnh
    pub hot: Option<(Tex, [f32; 4])>,
    /// khung chọn thanh nhanh
    pub sel: Option<(Tex, [f32; 4])>,
    /// Steve toàn thân (đơn vị điểm ảnh model) cho khung nhân vật trong túi đồ
    pub body_v: Vec<V>,
    pub body_i: Vec<u32>,
    pub v: Vec<V>,
    pub i: Vec<u32>,
    pub list: Vec<Item>,
    pub skin: Skin,
    pub arm_v: Vec<V>,
    pub arm_i: Vec<u32>,
}

fn pretty(key: &str) -> String {
    let mut s = key.replace('_', " ");
    if let Some(c) = s.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    s
}

fn load_skin(jar: &Jar) -> Skin {
    for p in ["assets/minecraft/textures/entity/player/wide/steve.png", "assets/minecraft/textures/entity/steve.png"] {
        if let Ok(b) = jar.read(p) {
            if let Ok(img) = png::decode(&b) {
                return Skin { rgba: img.rgba, w: img.w as i32, h: img.h as i32 };
            }
        }
    }
    // không tìm thấy skin: màu da trơn
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    for _ in 0..64 * 64 {
        rgba.extend_from_slice(&[190, 133, 100, 255]);
    }
    Skin { rgba, w: 64, h: 64 }
}

/// Thêm một hộp kiểu ModelPart: góc [mn, mx] (điểm ảnh model, đã gồm nở ra), gốc ảnh (u, v), kích thước ảnh (bw, bh, bd), dịch `off`, chia `div`.
fn add_box(v: &mut Vec<V>, idx: &mut Vec<u32>, tw: f32, th: f32, mn: [f32; 3], mx: [f32; 3], uv: (f32, f32), d: (f32, f32, f32), off: [f32; 3], div: f32) {
    let (x0, y0, z0) = (mn[0], mn[1], mn[2]);
    let (x1, y1, z1) = (mx[0], mx[1], mx[2]);
    let (u, vv) = uv;
    let (bw, bh, bd) = d;
    // (góc A, trục u, trục v, vùng ảnh u0,v0,u1,v1)
    let faces: [([f32; 3], [f32; 3], [f32; 3], [f32; 4]); 6] = [
        // mặt trên (-y): v tăng về phía -z
        ([x0, y0, z1], [x1 - x0, 0.0, 0.0], [0.0, 0.0, z0 - z1], [u + bd, vv, u + bd + bw, vv + bd]),
        // mặt dưới (+y)
        ([x0, y1, z0], [x1 - x0, 0.0, 0.0], [0.0, 0.0, z1 - z0], [u + bd + bw, vv, u + bd + 2.0 * bw, vv + bd]),
        // trước (-z)
        ([x0, y0, z0], [x1 - x0, 0.0, 0.0], [0.0, y1 - y0, 0.0], [u + bd, vv + bd, u + bd + bw, vv + bd + bh]),
        // sau (+z)
        ([x1, y0, z1], [x0 - x1, 0.0, 0.0], [0.0, y1 - y0, 0.0], [u + 2.0 * bd + bw, vv + bd, u + 2.0 * bd + 2.0 * bw, vv + bd + bh]),
        // ngoài (-x)
        ([x0, y0, z1], [0.0, 0.0, z0 - z1], [0.0, y1 - y0, 0.0], [u, vv + bd, u + bd, vv + bd + bh]),
        // trong (+x)
        ([x1, y0, z0], [0.0, 0.0, z1 - z0], [0.0, y1 - y0, 0.0], [u + bd + bw, vv + bd, u + 2.0 * bd + bw, vv + bd + bh]),
    ];
    for (a, du, dv, px) in faces {
        let base = v.len() as u32;
        let e = 0.01;
        let uvs = [(px[0] + e, px[1] + e), (px[2] - e, px[1] + e), (px[2] - e, px[3] - e), (px[0] + e, px[3] - e)];
        for (k, (s, t)) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].iter().enumerate() {
            let p = [(a[0] + du[0] * s + dv[0] * t + off[0]) / div, (a[1] + du[1] * s + dv[1] * t + off[1]) / div, (a[2] + du[2] * s + dv[2] * t + off[2]) / div];
            v.push(V { p, uv: [uvs[k].0 / tw, uvs[k].1 / th], c: [255; 4] });
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Tay phải của Steve (tay to 4 điểm ảnh) cộng lớp áo ngoài, theo toạ độ model của Minecraft (gốc xoay ở vai), đơn vị khối.
fn arm_mesh(w: i32, h: i32) -> (Vec<V>, Vec<u32>) {
    let (tw, th) = (w as f32, h as f32);
    let (mut v, mut idx) = (Vec::new(), Vec::new());
    let pivot = [-5.0f32, 2.0, 0.0];
    add_box(&mut v, &mut idx, tw, th, [-3.0, -2.0, -2.0], [1.0, 10.0, 2.0], (40.0, 16.0), (4.0, 12.0, 4.0), pivot, 16.0);
    if h >= 64 {
        add_box(&mut v, &mut idx, tw, th, [-3.25, -2.25, -2.25], [1.25, 10.25, 2.25], (40.0, 32.0), (4.0, 12.0, 4.0), pivot, 16.0);
    }
    (v, idx)
}

/// Steve toàn thân đứng thẳng, đơn vị điểm ảnh model (y hướng xuống, đầu y -8..0, chân chạm y = 24).
fn body_mesh(w: i32, h: i32) -> (Vec<V>, Vec<u32>) {
    let (tw, th) = (w as f32, h as f32);
    let new = h >= 64;
    let (mut v, mut idx) = (Vec::new(), Vec::new());
    let mut part = |mn: [f32; 3], mx: [f32; 3], uv: (f32, f32), d: (f32, f32, f32), off: [f32; 3], grow: f32| {
        add_box(&mut v, &mut idx, tw, th, [mn[0] - grow, mn[1] - grow, mn[2] - grow], [mx[0] + grow, mx[1] + grow, mx[2] + grow], uv, d, off, 1.0);
    };
    let z = [0.0f32; 3];
    // (gốc ảnh thân, gốc ảnh lớp ngoài) — skin cũ 64x32 dùng chung tay/chân phải cho bên trái, không có lớp ngoài trừ mũ
    part([-4.0, -8.0, -4.0], [4.0, 0.0, 4.0], (0.0, 0.0), (8.0, 8.0, 8.0), z, 0.0);
    part([-4.0, -8.0, -4.0], [4.0, 0.0, 4.0], (32.0, 0.0), (8.0, 8.0, 8.0), z, 0.5);
    part([-4.0, 0.0, -2.0], [4.0, 12.0, 2.0], (16.0, 16.0), (8.0, 12.0, 4.0), z, 0.0);
    part([-3.0, -2.0, -2.0], [1.0, 10.0, 2.0], (40.0, 16.0), (4.0, 12.0, 4.0), [-5.0, 2.0, 0.0], 0.0);
    part([-1.0, -2.0, -2.0], [3.0, 10.0, 2.0], if new { (32.0, 48.0) } else { (40.0, 16.0) }, (4.0, 12.0, 4.0), [5.0, 2.0, 0.0], 0.0);
    part([-2.0, 0.0, -2.0], [2.0, 12.0, 2.0], (0.0, 16.0), (4.0, 12.0, 4.0), [-1.9, 12.0, 0.0], 0.0);
    part([-2.0, 0.0, -2.0], [2.0, 12.0, 2.0], if new { (16.0, 48.0) } else { (0.0, 16.0) }, (4.0, 12.0, 4.0), [1.9, 12.0, 0.0], 0.0);
    if new {
        part([-4.0, 0.0, -2.0], [4.0, 12.0, 2.0], (16.0, 32.0), (8.0, 12.0, 4.0), z, 0.25);
        part([-3.0, -2.0, -2.0], [1.0, 10.0, 2.0], (40.0, 32.0), (4.0, 12.0, 4.0), [-5.0, 2.0, 0.0], 0.25);
        part([-1.0, -2.0, -2.0], [3.0, 10.0, 2.0], (48.0, 48.0), (4.0, 12.0, 4.0), [5.0, 2.0, 0.0], 0.25);
        part([-2.0, 0.0, -2.0], [2.0, 12.0, 2.0], (0.0, 32.0), (4.0, 12.0, 4.0), [-1.9, 12.0, 0.0], 0.25);
        part([-2.0, 0.0, -2.0], [2.0, 12.0, 2.0], (0.0, 48.0), (4.0, 12.0, 4.0), [1.9, 12.0, 0.0], 0.25);
    }
    (v, idx)
}

fn load_tex(jar: &Jar, paths: &[&str]) -> Option<Tex> {
    for p in paths {
        if let Ok(b) = jar.read(p) {
            if let Ok(img) = png::decode(&b) {
                return Some(Tex { rgba: img.rgba, w: img.w as i32, h: img.h as i32 });
            }
        }
    }
    None
}

fn load_hud(jar: &Jar) -> Option<(Hud, Vec<Tex>)> {
    let names = ["heart/container", "heart/full", "heart/half", "food_empty", "food_full", "food_half", "experience_bar_background"];
    let mut texs = Vec::new();
    for n in names {
        texs.push(load_tex(jar, &[&format!("assets/minecraft/textures/gui/sprites/hud/{n}.png")])?);
    }
    let sp = |i: usize| Spr { tex: i, r: [0.0, 0.0, texs[i].w as f32, texs[i].h as f32] };
    let hud = Hud { heart_bg: sp(0), heart_full: sp(1), heart_half: sp(2), food_bg: sp(3), food_full: sp(4), food_half: sp(5), xp_bg: sp(6) };
    Some((hud, texs))
}

/// HUD kiểu cũ: mọi biểu tượng nằm trong icons.png (256x256).
fn load_hud_old(jar: &Jar) -> Option<(Hud, Vec<Tex>)> {
    let t = load_tex(jar, &["assets/minecraft/textures/gui/icons.png"])?;
    let s = |x: f32, y: f32, w: f32, h: f32| Spr { tex: 0, r: [x, y, w, h] };
    let hud = Hud {
        heart_bg: s(16.0, 0.0, 9.0, 9.0),
        heart_full: s(52.0, 0.0, 9.0, 9.0),
        heart_half: s(61.0, 0.0, 9.0, 9.0),
        food_bg: s(16.0, 27.0, 9.0, 9.0),
        food_full: s(52.0, 27.0, 9.0, 9.0),
        food_half: s(61.0, 27.0, 9.0, 9.0),
        xp_bg: s(0.0, 64.0, 182.0, 5.0),
    };
    Some((hud, vec![t]))
}

/// Tìm các vật phẩm trong tag (đệ quy các tag lồng nhau).
fn tag_items(jar: &Jar, name: &str, depth: u32, out: &mut Vec<String>) {
    if depth > 6 {
        return;
    }
    let n = name.trim_start_matches('#').trim_start_matches("minecraft:");
    for dir in ["items", "item"] {
        let Ok(b) = jar.read(&format!("data/minecraft/tags/{dir}/{n}.json")) else { continue };
        let Ok(text) = String::from_utf8(b) else { continue };
        let Ok(v) = json::parse(&text) else { continue };
        for e in v.get("values").and_then(|a| a.as_arr()).unwrap_or(&[]) {
            let id = match e {
                Value::Str(s) => Some(s.as_str()),
                other => other.get("id").and_then(|i| i.as_str()),
            };
            if let Some(id) = id {
                if id.starts_with('#') {
                    tag_items(jar, id, depth + 1, out);
                } else {
                    out.push(id.to_string());
                }
            }
        }
        return;
    }
}

/// Một nguyên liệu (chuỗi "minecraft:x" / "#tag", {"item"|"tag"|"id"}, hoặc mảng các lựa chọn) -> các chỉ số vật phẩm của ta.
fn ingredient(jar: &Jar, v: &Value, idx: &HashMap<String, u16>, out: &mut Vec<u16>) {
    match v {
        Value::Str(s) => {
            if s.starts_with('#') {
                let mut names = Vec::new();
                tag_items(jar, s, 0, &mut names);
                for n in names {
                    if let Some(&i) = idx.get(&n) {
                        out.push(i);
                    }
                }
            } else if let Some(&i) = idx.get(s.as_str()) {
                out.push(i);
            }
        }
        Value::Arr(a) => a.iter().for_each(|e| ingredient(jar, e, idx, out)),
        Value::Obj(_) => {
            if let Some(t) = v.get("tag").and_then(|t| t.as_str()) {
                ingredient(jar, &Value::Str(format!("#{t}")), idx, out);
            }
            for k in ["item", "id"] {
                if let Some(t) = v.get(k).and_then(|t| t.as_str()) {
                    ingredient(jar, &Value::Str(t.to_string()), idx, out);
                }
            }
        }
        _ => {}
    }
}

/// Công thức chế tạo có kết quả là một vật phẩm của ta (chỉ đọc file recipe trùng tên vật phẩm cho nhanh).
fn load_recipes(jar: &Jar, keys: &[String]) -> Vec<Recipe> {
    let idx: HashMap<String, u16> = keys.iter().enumerate().map(|(i, k)| (format!("minecraft:{k}"), i as u16)).collect();
    let mut out = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        for dir in ["recipes", "recipe"] {
            let path = format!("data/minecraft/{dir}/{k}.json");
            let Ok(b) = jar.read(&path) else { continue };
            let Ok(text) = String::from_utf8(b) else { continue };
            let Ok(v) = json::parse(&text) else { continue };
            let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let count = v.get("result").and_then(|r| r.get("count")).and_then(|c| c.as_f64()).unwrap_or(1.0) as u8;
            let out_item = (i as u16, count.max(1));
            if ty.ends_with("crafting_shaped") {
                let (Some(pat), Some(key)) = (v.get("pattern").and_then(|p| p.as_arr()), v.get("key")) else { continue };
                let rows: Vec<&str> = pat.iter().filter_map(|r| r.as_str()).collect();
                let (h, w) = (rows.len(), rows.iter().map(|r| r.chars().count()).max().unwrap_or(0));
                if h == 0 || w == 0 || h > 2 || w > 2 {
                    continue;
                }
                let mut cells = Vec::new();
                let mut ok = true;
                for r in &rows {
                    let mut chars: Vec<char> = r.chars().collect();
                    chars.resize(w, ' ');
                    for c in chars {
                        let mut opts = Vec::new();
                        if c != ' ' {
                            if let Some(ing) = key.get(&c.to_string()) {
                                ingredient(jar, ing, &idx, &mut opts);
                            }
                            if opts.is_empty() {
                                ok = false;
                            }
                        }
                        cells.push(opts);
                    }
                }
                if ok {
                    out.push(Recipe { shaped: true, w, h, cells, out: out_item });
                }
            } else if ty.ends_with("crafting_shapeless") {
                let Some(ings) = v.get("ingredients").and_then(|a| a.as_arr()) else { continue };
                if ings.is_empty() || ings.len() > 4 {
                    continue;
                }
                let mut cells = Vec::new();
                let mut ok = true;
                for ing in ings {
                    let mut opts = Vec::new();
                    ingredient(jar, ing, &idx, &mut opts);
                    if opts.is_empty() {
                        ok = false;
                    }
                    cells.push(opts);
                }
                if ok {
                    out.push(Recipe { shaped: false, w: 0, h: 0, cells, out: out_item });
                }
            }
            break;
        }
    }
    out
}

/// Tìm tên model đầu tiên trong định nghĩa items/<tên>.json (có thể lồng select/condition/composite...).
fn find_model(v: &Value) -> Option<String> {
    if let Some(m) = v.get("model") {
        match m {
            Value::Str(s) => return Some(s.clone()),
            Value::Obj(_) => {
                if let Some(r) = find_model(m) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    if let Some(Value::Str(s)) = v.get("base") {
        return Some(s.clone());
    }
    for k in ["fallback", "on_false", "on_true"] {
        if let Some(r) = v.get(k).and_then(find_model) {
            return Some(r);
        }
    }
    for k in ["cases", "entries", "models"] {
        if let Some(a) = v.get(k).and_then(|a| a.as_arr()) {
            for e in a {
                if let Some(r) = find_model(e) {
                    return Some(r);
                }
            }
        }
    }
    None
}

fn wear_slot(name: &str) -> i8 {
    if name.ends_with("_helmet") || name == "carved_pumpkin" {
        0
    } else if name.ends_with("_chestplate") {
        1
    } else if name.ends_with("_leggings") {
        2
    } else if name.ends_with("_boots") {
        3
    } else {
        -1
    }
}

/// Danh sách vật phẩm theo ĐÚNG số id mạng của server (netitems.txt): khối có icon 3D, còn lại là sprite phẳng.
fn build_net_list(jar: &Jar, atlas: &Atlas, m: &mut Mesh, lang: &Option<Value>) -> (Vec<Item>, Vec<i8>) {
    let a_f = crate::assets::ATLAS as f32;
    let tile = crate::assets::TILE as f32;
    let mut list = Vec::new();
    let mut wear = Vec::new();
    let mut flat_n = 0;
    let mut cube_n = 0;
    for name in include_str!("netitems.txt").lines() {
        let disp = lang
            .as_ref()
            .and_then(|l| l.get(&format!("item.minecraft.{name}")).or_else(|| l.get(&format!("block.minecraft.{name}"))))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| pretty(name));
        wear.push(wear_slot(name));
        let mut item = Item { name: disp, gui: (0, 0), hand: (0, 0), flat: false };
        if name == "air" {
            list.push(item);
            continue;
        }
        let mname = jar
            .read(&format!("assets/minecraft/items/{name}.json"))
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|t| json::parse(&t).ok())
            .and_then(|v| find_model(&v))
            .map(|s| s.trim_start_matches("minecraft:").to_string())
            .unwrap_or_else(|| format!("item/{name}"));
        let model = load_model(jar, &mname, 0).ok().or_else(|| load_model(jar, &format!("block/{name}"), 0).ok());
        if let Some(md) = model.as_ref().filter(|md| !md.elements.is_empty()) {
            let faces = bake(md, atlas, 0, 0);
            if !faces.is_empty() {
                let tint = crate::blocks::tint_for(name);
                let col = |f: &BFace| if f.tint { tint } else { [255u8; 3] };
                let a = m.i.len() as i32;
                for f in &faces {
                    let mut g = f.clone();
                    g.shade = if (f.shade - 0.6).abs() < 0.01 { 0.8 } else if (f.shade - 0.8).abs() < 0.01 { 0.6 } else { f.shade };
                    m.push_face(&g, [0.0; 3], col(f));
                }
                let b = m.i.len() as i32;
                for f in &faces {
                    m.push_face(f, [0.0; 3], col(f));
                }
                let c = m.i.len() as i32;
                item.gui = (a, b - a);
                item.hand = (b, c - b);
                cube_n += 1;
                list.push(item);
                continue;
            }
        }
        // sprite phẳng: các lớp layer0, layer1... của model
        let mut layers: Vec<(usize, usize)> = Vec::new();
        if let Some(md) = &model {
            for k in 0..4 {
                if let Some(t) = md.textures.get(&format!("layer{k}")) {
                    let t = t.trim_start_matches("minecraft:");
                    if let Some(&p) = atlas.tiles.get(t) {
                        layers.push(p);
                    }
                }
            }
        }
        if layers.is_empty() {
            if let Some(&p) = atlas.tiles.get(&format!("item/{name}")).or_else(|| atlas.tiles.get(name)) {
                layers.push(p);
            }
        }
        if !layers.is_empty() {
            let a = m.i.len() as i32;
            for (li, (tx, ty)) in layers.iter().enumerate() {
                let (u0, v0) = (*tx as f32 * tile / a_f, *ty as f32 * tile / a_f);
                let (u1, v1) = (u0 + tile / a_f, v0 + tile / a_f);
                let z = 0.5 + li as f32 * 0.002;
                let base = m.v.len() as u32;
                for (p, uv) in [([0.0, 0.0], [u0, v1]), ([1.0, 0.0], [u1, v1]), ([1.0, 1.0], [u1, v0]), ([0.0, 1.0], [u0, v0])] {
                    m.v.push(V { p: [p[0], p[1], z], uv, c: [255; 4] });
                }
                m.i.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            let b = m.i.len() as i32;
            item.gui = (a, b - a);
            item.hand = (a, b - a);
            item.flat = true;
            flat_n += 1;
        }
        list.push(item);
    }
    let _ = (flat_n, cube_n);
    (list, wear)
}

pub fn build(jar: &Jar, atlas: &Atlas, log: &dyn Fn(String)) -> Items {
    build_any(jar, atlas, log, false)
}

/// Bản cho nhiều người chơi: danh sách theo id vật phẩm mạng, không công thức chế tạo.
pub fn build_net(jar: &Jar, atlas: &Atlas, log: &dyn Fn(String)) -> Items {
    build_any(jar, atlas, log, true)
}

fn build_any(jar: &Jar, atlas: &Atlas, log: &dyn Fn(String), net: bool) -> Items {
    let mut res = Resolver::new(jar, atlas);
    let lang = jar.read("assets/minecraft/lang/en_us.json").ok().and_then(|b| String::from_utf8(b).ok()).and_then(|s| json::parse(&s).ok());
    let mut m = Mesh::default();
    let mut list = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let mut net_wear: Vec<i8> = Vec::new();
    if net {
        let (l, w) = build_net_list(jar, atlas, &mut m, &lang);
        list = l;
        net_wear = w;
    }
    for key in KEYS.iter().copied().filter(|_| !net) {
        let st = res.state(&format!("minecraft:{key}"));
        if st.faces.is_empty() {
            continue;
        }
        let col = |f: &BFace| if f.tint { st.tint } else { [255u8; 3] };
        let a = m.i.len() as i32;
        // icon GUI: ánh sáng phẳng kiểu Minecraft (mặt trên 1.0, trái 0.8, phải 0.6)
        for f in &st.faces {
            let mut g = f.clone();
            g.shade = if (f.shade - 0.6).abs() < 0.01 { 0.8 } else if (f.shade - 0.8).abs() < 0.01 { 0.6 } else { f.shade };
            m.push_face(&g, [0.0; 3], col(f));
        }
        let b = m.i.len() as i32;
        for f in &st.faces {
            m.push_face(f, [0.0; 3], col(f));
        }
        let c = m.i.len() as i32;
        let name = lang.as_ref().and_then(|l| l.get(&format!("block.minecraft.{key}"))).and_then(|n| n.as_str()).map(|s| s.to_string()).unwrap_or_else(|| pretty(key));
        list.push(Item { name, gui: (a, b - a), hand: (b, c - b), flat: false });
        keys.push(key.to_string());
    }
    let skin = load_skin(jar);
    let (arm_v, arm_i) = arm_mesh(skin.w, skin.h);
    let (body_v, body_i) = body_mesh(skin.w, skin.h);
    let inv = load_tex(jar, &["assets/minecraft/textures/gui/container/inventory.png"]);
    let chest = load_tex(jar, &["assets/minecraft/textures/gui/container/generic_54.png"]);
    // thanh nhanh: ảnh riêng (1.20.2+) hoặc widgets.png (cũ hơn)
    let hot = load_tex(jar, &["assets/minecraft/textures/gui/sprites/hud/hotbar.png"]).map(|t| (t, [0.0, 0.0, 182.0, 22.0])).or_else(|| load_tex(jar, &["assets/minecraft/textures/gui/widgets.png"]).map(|t| (t, [0.0, 0.0, 182.0, 22.0])));
    let sel = load_tex(jar, &["assets/minecraft/textures/gui/sprites/hud/hotbar_selection.png"]).map(|t| {
        let r = [0.0, 0.0, t.w as f32, t.h as f32];
        (t, r)
    }).or_else(|| load_tex(jar, &["assets/minecraft/textures/gui/widgets.png"]).map(|t| (t, [0.0, 22.0, 24.0, 24.0])));
    let recipes = if net { Vec::new() } else { load_recipes(jar, &keys) };
    let wear: Vec<i8> = if net { net_wear } else { keys.iter().map(|k| if k == "carved_pumpkin" { 0 } else { -1 }).collect() };
    let (hud, hud_tex) = match load_hud(jar).or_else(|| load_hud_old(jar)) {
        Some((h, t)) => (Some(h), t),
        None => (None, Vec::new()),
    };
    log(format!("Công thức 2x2: {}, HUD: {}", recipes.len(), hud.is_some()));
    log(format!("GUI: túi đồ {}, thanh nhanh {}, khung chọn {}", inv.is_some(), hot.is_some(), sel.is_some()));
    log(format!("Vật phẩm: {} khối, skin {}x{}", list.len(), skin.w, skin.h));
    Items { hud, hud_tex, head_n: 72, wear, recipes, inv, chest, hot, sel, body_v, body_i, v: m.v, i: m.i, list, skin, arm_v, arm_i }
}

/// Thời gian một nhịp vung tay (6 tick = 0.3 giây như Minecraft).
pub const SWING_T: f32 = 0.3;

/// Tay trống: giống hệt ItemInHandRenderer.renderPlayerArm của Minecraft (tay phải). `lower` 0 = nâng hẳn, 1 = hạ hẳn; `sw` tiến độ vung 0..1.
pub fn arm_matrix(lower: f32, sw: f32) -> Mat4 {
    let f1 = sw.sqrt();
    let f2 = -0.3 * (f1 * PI).sin();
    let f3 = 0.4 * (f1 * 2.0 * PI).sin();
    let f4 = -0.4 * (sw * PI).sin();
    let f5 = (sw * sw * PI).sin();
    let f6 = (f1 * PI).sin();
    math::chain(&[
        math::translate(f2 + 0.64000005, f3 - 0.6 - lower * 0.6, f4 - 0.71999997),
        math::rot_y(45f32.to_radians()),
        math::rot_y((f6 * 70.0).to_radians()),
        math::rot_z((f5 * -20.0).to_radians()),
        math::translate(-1.0, 3.6, 3.5),
        math::rot_z(120f32.to_radians()),
        math::rot_x(200f32.to_radians()),
        math::rot_y((-135f32).to_radians()),
        math::translate(5.6, 0.0, 0.0),
    ])
}

/// Khối cầm trên tay: dịch/xoay như renderArmWithItem + transform "firstperson_righthand" của model khối (xoay Y 45°, co 0.4), mô hình 0..1 tâm ở (.5,.5,.5).
pub fn hand_matrix(lower: f32, sw: f32) -> Mat4 {
    let sq = sw.sqrt();
    let f5 = -0.4 * (sq * PI).sin();
    let f6 = 0.2 * (sq * 2.0 * PI).sin();
    let f10 = -0.2 * (sw * PI).sin();
    let f = (sw * sw * PI).sin();
    let f1 = (sq * PI).sin();
    math::chain(&[
        math::translate(f5, f6, f10),
        math::translate(0.56, -0.52 - lower * 0.6, -0.71999997),
        math::rot_y((45.0 - f * 20.0).to_radians()),
        math::rot_z((f1 * -20.0).to_radians()),
        math::rot_x((f1 * -80.0).to_radians()),
        math::rot_y((-45f32).to_radians()),
        math::rot_y(45f32.to_radians()),
        math::scale(0.4, 0.4, 0.4),
        math::translate(-0.5, -0.5, -0.5),
    ])
}

/// Icon khối trong GUI (transform "gui" của model khối: xoay X 30°, Y 225°, co 0.625) đặt tại (cx, cy) pixel, `sz` pixel / khối.
pub fn gui_matrix(w: f32, h: f32, cx: f32, cy: f32, sz: f32, zoff: f32) -> Mat4 {
    let ortho: Mat4 = [2.0 / w, 0.0, 0.0, 0.0, 0.0, -2.0 / h, 0.0, 0.0, 0.0, 0.0, -1.0 / 4000.0, 0.0, -1.0, 1.0, 0.0, 1.0];
    math::chain(&[
        ortho,
        math::translate(cx, cy, zoff),
        math::scale(sz, -sz, sz),
        math::rot_x(30f32.to_radians()),
        math::rot_y(225f32.to_radians()),
        math::scale(0.625, 0.625, 0.625),
        math::translate(-0.5, -0.5, -0.5),
    ])
}

/// Icon phẳng (sprite) trong GUI: hình vuông đơn vị 0..1 đặt tại (cx, cy), `sz` pixel mỗi cạnh, không xoay.
pub fn flat_matrix(w: f32, h: f32, cx: f32, cy: f32, sz: f32, zoff: f32) -> Mat4 {
    math::chain(&[ortho(w, h), math::translate(cx, cy, zoff), math::scale(sz, -sz, sz), math::translate(-0.5, -0.5, -0.5)])
}

/// Phép chiếu trực giao theo pixel màn hình (gốc trên-trái), z dương = gần người xem.
pub fn ortho(w: f32, h: f32) -> Mat4 {
    [2.0 / w, 0.0, 0.0, 0.0, 0.0, -2.0 / h, 0.0, 0.0, 0.0, 0.0, -1.0 / 4000.0, 0.0, -1.0, 1.0, 0.0, 1.0]
}

/// Steve trong khung nhân vật: chân ở (cx, feet), `ks` pixel mỗi điểm ảnh model. `body` xoay thân quanh Y, `head`/`pitch` xoay đầu thêm (radian).
/// Trả về (ma trận thân, ma trận đầu).
pub fn body_matrices(w: f32, h: f32, cx: f32, feet: f32, ks: f32, body: f32, head: f32, pitch: f32) -> (Mat4, Mat4) {
    let base = math::chain(&[ortho(w, h), math::translate(cx, feet - 24.0 * ks, 0.0), math::scale(ks, ks, -ks), math::rot_y(body)]);
    let hd = math::chain(&[base, math::rot_y(head), math::rot_x(pitch)]);
    (base, hd)
}
