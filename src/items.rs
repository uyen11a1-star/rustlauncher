//! Vật phẩm cầm tay và ô đồ: mesh khối cho icon GUI / khối cầm tay, tay Steve góc nhìn thứ nhất, ma trận hoạt ảnh như Minecraft.
use std::f32::consts::PI;

use crate::{
    assets::{Atlas, BFace, Jar, Mesh, V},
    blocks::Resolver,
    json,
    math::{self, Mat4},
    png,
};

/// Các khối đưa vào túi đồ (bỏ qua khối nào không dựng được).
const KEYS: [&str; 36] = [
    "stone", "dirt", "grass_block", "cobblestone", "oak_planks", "oak_log", "sand", "glass", "oak_leaves", "gravel", "bricks", "bookshelf", "crafting_table", "furnace", "tnt", "stone_bricks", "sandstone", "obsidian", "glowstone",
    "white_wool", "red_wool", "birch_planks", "spruce_planks", "birch_log", "spruce_log", "gold_block", "iron_block", "diamond_block", "coal_ore", "iron_ore", "snow_block", "clay", "netherrack", "pumpkin", "melon", "bedrock",
];

pub struct Item {
    pub name: String,
    /// (chỉ số đầu, số chỉ số) trong `Items::i`: bản vẽ icon GUI / bản cầm tay
    pub gui: (i32, i32),
    pub hand: (i32, i32),
}

pub struct Skin {
    pub rgba: Vec<u8>,
    pub w: i32,
    pub h: i32,
}

pub struct Items {
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

/// Tay phải của Steve (tay to 4 điểm ảnh) cộng lớp áo ngoài, theo toạ độ model của Minecraft (gốc xoay ở vai), đơn vị khối.
fn arm_mesh(w: i32, h: i32) -> (Vec<V>, Vec<u32>) {
    let (tw, th) = (w as f32, h as f32);
    let mut v: Vec<V> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    let pivot = [-5.0f32, 2.0, 0.0];
    let mut boxes: Vec<(f32, f32, f32, f32)> = vec![(0.0, 40.0, 16.0, 0.0)]; // (nở ra, U, V, _)
    if h >= 64 {
        boxes.push((0.25, 40.0, 32.0, 0.0));
    }
    for (grow, u, vv, _) in boxes {
        let (x0, y0, z0) = (-3.0 - grow, -2.0 - grow, -2.0 - grow);
        let (x1, y1, z1) = (1.0 + grow, 10.0 + grow, 2.0 + grow);
        let (bw, bh, bd) = (4.0f32, 12.0f32, 4.0f32);
        // (góc A, trục u, trục v, vùng ảnh u0,v0,u1,v1)
        let faces: [([f32; 3], [f32; 3], [f32; 3], [f32; 4]); 6] = [
            // mặt trên (-y): v tăng về phía -z
            ([x0, y0, z1], [x1 - x0, 0.0, 0.0], [0.0, 0.0, z0 - z1], [u + bd, vv, u + bd + bw, vv + bd]),
            // mặt dưới (+y): đầu nắm tay
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
                let p = [
                    (a[0] + du[0] * s + dv[0] * t + pivot[0]) / 16.0,
                    (a[1] + du[1] * s + dv[1] * t + pivot[1]) / 16.0,
                    (a[2] + du[2] * s + dv[2] * t + pivot[2]) / 16.0,
                ];
                v.push(V { p, uv: [uvs[k].0 / tw, uvs[k].1 / th], c: [255; 4] });
            }
            idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    (v, idx)
}

pub fn build(jar: &Jar, atlas: &Atlas, log: &dyn Fn(String)) -> Items {
    let mut res = Resolver::new(jar, atlas);
    let lang = jar.read("assets/minecraft/lang/en_us.json").ok().and_then(|b| String::from_utf8(b).ok()).and_then(|s| json::parse(&s).ok());
    let mut m = Mesh::default();
    let mut list = Vec::new();
    for key in KEYS {
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
        list.push(Item { name, gui: (a, b - a), hand: (b, c - b) });
    }
    let skin = load_skin(jar);
    let (arm_v, arm_i) = arm_mesh(skin.w, skin.h);
    log(format!("Vật phẩm: {} khối, skin {}x{}", list.len(), skin.w, skin.h));
    Items { v: m.v, i: m.i, list, skin, arm_v, arm_i }
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
