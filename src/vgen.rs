//! Sinh chunk bằng bộ máy worldgen ở chỗ thế giới chưa được tạo: địa hình, nước biển và lớp bề mặt
//! (chưa có hang, quặng, cây, công trình). Trả về `RawChunk` giống chunk đọc từ đĩa nên đi chung đường nạp.
use std::collections::HashMap;

use crate::{
    vdens::{steep_at, Router},
    world::{RawChunk, RawSection},
};

const MIN_Y: i32 = -64;
const H: usize = 384;
const SEC_LO: i32 = -4;
const SEC_HI: i32 = 19;

/// Khóa block state đầy đủ thuộc tính mặc định, giống palette trong file thế giới.
fn key(name: &str) -> String {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let props = match n {
        "grass_block" | "podzol" | "mycelium" => "[snowy=false]",
        "water" | "lava" => "[level=0]",
        "deepslate" | "basalt" | "polished_basalt" => "[axis=y]",
        "snow" => "[layers=1]",
        _ => "",
    };
    format!("minecraft:{n}{props}")
}

pub fn generate(r: &Router, cx: i32, cz: i32) -> RawChunk {
    let (x0, z0) = (cx * 16, cz * 16);
    r.prepare(x0, z0);
    let sea = r.sea_level();
    let mut solid: Vec<Vec<bool>> = Vec::with_capacity(256);
    let mut top = [[MIN_Y; 16]; 16];
    for lz in 0..16usize {
        for lx in 0..16usize {
            let (x, z) = (x0 + lx as i32, z0 + lz as i32);
            let col: Vec<bool> = (0..H).map(|i| r.density(x, MIN_Y + i as i32, z) > 0.0).collect();
            let mut t = MIN_Y;
            for i in (0..H).rev() {
                if col[i] || MIN_Y + (i as i32) < sea {
                    t = MIN_Y + i as i32 + 1;
                    break;
                }
            }
            top[lz][lx] = t;
            solid.push(col);
        }
    }
    // mã khối từng cột và ánh sáng trời đơn giản theo cột: 15 trên mặt, trừ 1 mỗi khối nước, 0 dưới đất đặc
    let mut ids: Vec<Vec<u16>> = Vec::with_capacity(256);
    let mut sky: Vec<Vec<u8>> = Vec::with_capacity(256);
    for lz in 0..16usize {
        for lx in 0..16usize {
            let tp = |a: i32, b: i32| top[b as usize][a as usize];
            let steep = steep_at(&tp, lx as i32, lz as i32);
            let col = r.surface_column(x0 + lx as i32, z0 + lz as i32, &solid[lz * 16 + lx], steep);
            let mut s = vec![0u8; H];
            let mut lvl = 15u8;
            for i in (0..H).rev() {
                match col[i] {
                    0 => s[i] = lvl,
                    1 => {
                        lvl = lvl.saturating_sub(1);
                        s[i] = lvl;
                    }
                    _ => lvl = 0,
                }
            }
            ids.push(col);
            sky.push(s);
        }
    }
    let mut secs = Vec::new();
    let mut lights = Vec::new();
    for sy in SEC_LO..=SEC_HI {
        let base = (sy * 16 - MIN_Y) as usize;
        let mut pal: Vec<String> = Vec::new();
        let mut map: HashMap<u16, u16> = HashMap::new();
        let mut idx = vec![0u16; 4096];
        let mut sk = vec![0u8; 2048];
        let mut any = false;
        for ly in 0..16usize {
            for lz in 0..16usize {
                for lx in 0..16usize {
                    let id = ids[lz * 16 + lx][base + ly];
                    let i = (ly << 8) | (lz << 4) | lx;
                    let p = *map.entry(id).or_insert_with(|| {
                        pal.push(match id {
                            0 => "minecraft:air".to_string(),
                            1 => key("water"),
                            2 => key("stone"),
                            n => key(r.block_name(n)),
                        });
                        (pal.len() - 1) as u16
                    });
                    idx[i] = p;
                    if id != 0 {
                        any = true;
                    }
                    let l = sky[lz * 16 + lx][base + ly];
                    sk[i >> 1] |= if i & 1 == 0 { l } else { l << 4 };
                }
            }
        }
        if !any {
            continue;
        }
        lights.push((sy, Some(sk), None));
        secs.push(RawSection { y: sy, idx: if pal.len() == 1 { None } else { Some(idx) }, palette: pal });
    }
    RawChunk { cx, cz, secs, lights, biomes: Vec::new() }
}
