//! Cảnh thử G1: sàn cỏ + một dãy khối dựng từ model/texture THẬT trong client.jar.
use std::path::Path;

use crate::assets::{build_atlas, emit, load_model, Jar, V};

pub struct SceneData {
    pub atlas: Vec<u8>,
    pub verts: Vec<V>,
    pub spawn: [f32; 3],
}

const BLOCKS: [&str; 32] = [
    "stone", "dirt", "grass_block", "oak_planks", "oak_log", "cobblestone", "sand", "gravel", "bricks", "bookshelf", "crafting_table", "furnace", "tnt", "diamond_ore", "gold_block", "iron_block", "glass", "oak_leaves", "oak_slab", "oak_stairs", "white_wool", "red_wool",
    "sandstone", "obsidian", "netherrack", "glowstone", "pumpkin", "melon", "bedrock", "coal_ore", "lapis_block", "emerald_block",
];

pub fn build(jar_path: &Path, log: &dyn Fn(String)) -> Result<SceneData, String> {
    let t0 = std::time::Instant::now();
    log(format!("Mở {}", jar_path.display()));
    let jar = Jar::open(jar_path)?;
    log(format!("jar có {} mục", jar.entries.len()));
    let atlas = build_atlas(&jar, log)?;

    let mut verts: Vec<V> = Vec::new();
    // sàn
    let floor = load_model(&jar, "block/grass_block", 0)?;
    for x in -4..22 {
        for z in -14..8 {
            emit(&floor, &atlas, [x as f32, -1.0, z as f32], &mut verts);
        }
    }
    // dãy khối
    let mut shown = 0;
    for (i, name) in BLOCKS.iter().enumerate() {
        match load_model(&jar, &format!("block/{name}"), 0) {
            Ok(m) => {
                let at = [((i % 8) * 2) as f32, 0.0, -((i / 8) as f32 * 2.0) - 2.0];
                let before = verts.len();
                emit(&m, &atlas, at, &mut verts);
                if verts.len() == before {
                    log(format!("model {name}: không có mặt nào vẽ được"));
                } else {
                    shown += 1;
                }
            }
            Err(e) => log(format!("model {name}: {e}")),
        }
    }
    log(format!("Dựng {} khối, {} đỉnh, mất {} ms", shown, verts.len(), t0.elapsed().as_millis()));
    Ok(SceneData { atlas: atlas.pixels, verts, spawn: [7.0, 1.7, 4.0] })
}
