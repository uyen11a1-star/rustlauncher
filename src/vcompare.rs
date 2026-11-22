//! Công cụ so sánh: sinh địa hình, quần xã và lớp bề mặt bằng bộ máy rồi đối chiếu với chunk thật của thế giới.
use std::{collections::HashMap, path::Path};

use crate::{
    assets::Jar,
    vbiome::BiomeSource,
    vdens::{steep_at, Router},
    world::{self, World, UNKNOWN},
};

/// 1 = khối đặc tự nhiên, 2 = không khí hoặc chất lỏng, 0 = bỏ qua (cây, hoa, vật trang trí...).
fn class(key: &str) -> u8 {
    let n = key.split('[').next().unwrap_or(key);
    let n = n.strip_prefix("minecraft:").unwrap_or(n);
    match n {
        "air" | "cave_air" | "void_air" | "water" | "lava" => 2,
        "stone" | "deepslate" | "tuff" | "granite" | "diorite" | "andesite" | "dirt" | "grass_block" | "coarse_dirt" | "podzol" | "mycelium" | "rooted_dirt" | "sand"
        | "red_sand" | "sandstone" | "red_sandstone" | "gravel" | "clay" | "bedrock" | "snow_block" | "packed_ice" | "blue_ice" | "calcite" | "dripstone_block" | "mud"
        | "moss_block" | "terracotta" | "smooth_basalt" | "infested_stone" | "infested_deepslate" => 1,
        _ if n.ends_with("_ore") || n.ends_with("_terracotta") => 1,
        _ => 0,
    }
}

fn short(key: &str) -> &str {
    let n = key.split('[').next().unwrap_or(key);
    n.strip_prefix("minecraft:").unwrap_or(n)
}

fn pct(a: u64, b: u64) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 * 100.0 / b as f64
    }
}

const MIN_Y: i32 = -64;
const H: usize = 384;

pub fn compare(jar: &Jar, seed: i64, dir: &Path, w: &World, keys: &[String], chunks: &[(i32, i32)], log: &dyn Fn(String)) {
    let t0 = std::time::Instant::now();
    let router = match Router::build(jar, seed) {
        Ok(r) => r,
        Err(e) => {
            log(format!("So sánh địa hình: không dựng được bộ máy ({e})"));
            return;
        }
    };
    log(format!("So sánh địa hình: dựng bộ máy xong ({} ms)", t0.elapsed().as_millis()));
    let cls: Vec<u8> = keys.iter().map(|k| class(k)).collect();
    let sea = router.sea_level();
    let mut water_top: std::collections::BTreeMap<i32, u64> = std::collections::BTreeMap::new();
    let (mut tot, mut ok, mut extra, mut missing) = (0u64, 0u64, 0u64, 0u64);
    let (mut cols, mut col_ok, mut col_diff, mut col_both) = (0u64, 0u64, 0i64, 0u64);
    let (mut s_tot, mut s_ok, mut s_feat) = (0u64, 0u64, 0u64);
    let mut s_conf: HashMap<(String, String), u64> = HashMap::new();
    for &(cx, cz) in chunks {
        let (x0, z0) = (cx * 16, cz * 16);
        router.prepare(x0, z0);
        // mật độ cả chunk: mặt nạ đặc theo cột
        let mut solid: Vec<Vec<bool>> = Vec::with_capacity(256);
        let mut top = [[MIN_Y; 16]; 16];
        for lz in 0..16usize {
            for lx in 0..16usize {
                let (x, z) = (x0 + lx as i32, z0 + lz as i32);
                let col: Vec<bool> = (0..H).map(|i| router.density(x, MIN_Y + i as i32, z) > 0.0).collect();
                // độ cao mặt đất (kể cả mặt nước): khối không phải không khí cao nhất + 1
                let mut t = MIN_Y;
                for i in (0..H).rev() {
                    let y = MIN_Y + i as i32;
                    if col[i] || y < sea {
                        t = y + 1;
                        break;
                    }
                }
                top[lz][lx] = t;
                solid.push(col);
            }
        }
        for lz in 0..16usize {
            for lx in 0..16usize {
                let (x, z) = (x0 + lx as i32, z0 + lz as i32);
                let col = &solid[lz * 16 + lx];
                let (mut rtop, mut ptop) = (i32::MIN, i32::MIN);
                if let Some(wy) = (MIN_Y..=250).rev().find(|&y| {
                    let id = w.get(x, y, z);
                    id != UNKNOWN && short(&keys[id as usize]) == "water"
                }) {
                    *water_top.entry(wy).or_insert(0) += 1;
                }
                for y in (MIN_Y..=250).rev() {
                    let id = w.get(x, y, z);
                    if id == UNKNOWN {
                        continue;
                    }
                    let pred = col[(y - MIN_Y) as usize];
                    if pred && ptop == i32::MIN {
                        ptop = y;
                    }
                    match cls.get(id as usize).copied().unwrap_or(0) {
                        1 => {
                            tot += 1;
                            if pred {
                                ok += 1;
                            } else {
                                missing += 1;
                            }
                            if rtop == i32::MIN {
                                rtop = y;
                            }
                        }
                        2 => {
                            tot += 1;
                            if pred {
                                extra += 1;
                            } else {
                                ok += 1;
                            }
                        }
                        _ => {}
                    }
                }
                if rtop != i32::MIN || ptop != i32::MIN {
                    cols += 1;
                    if rtop == ptop {
                        col_ok += 1;
                    }
                    if rtop != i32::MIN && ptop != i32::MIN {
                        col_both += 1;
                        col_diff += (rtop as i64 - ptop as i64).abs();
                    }
                }
                // lớp bề mặt: so tên khối ở những ô cả hai bên đều đặc
                let tp = |a: i32, b: i32| top[b as usize][a as usize];
                let steep = steep_at(&tp, lx as i32, lz as i32);
                let pred_ids = router.surface_column(x, z, col, steep);
                for y in MIN_Y..=250 {
                    let id = w.get(x, y, z);
                    if id == UNKNOWN {
                        continue;
                    }
                    let pid = pred_ids[(y - MIN_Y) as usize];
                    if pid < 2 || cls.get(id as usize).copied().unwrap_or(0) != 1 {
                        continue;
                    }
                    let pn = router.block_name(pid);
                    let rn = short(&keys[id as usize]);
                    if (pn == "stone" || pn == "deepslate") && rn != "stone" && rn != "deepslate" {
                        s_feat += 1; // quặng, mạch đá, đất chen: do bước rải đặc điểm, không thuộc lớp bề mặt
                        continue;
                    }
                    s_tot += 1;
                    if pn == rn {
                        s_ok += 1;
                    } else {
                        *s_conf.entry((pn.to_string(), rn.to_string())).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    log(format!(
        "So sánh địa hình ({} chunk, {} ms): khớp {:.1}% trên {} khối; thừa đặc {}, thiếu đặc {}",
        chunks.len(),
        t0.elapsed().as_millis(),
        pct(ok, tot),
        tot,
        extra,
        missing
    ));
    log(format!(
        "Độ cao mặt đất: khớp đúng {:.1}% trên {} cột, lệch trung bình {:.2} khối",
        pct(col_ok, cols),
        cols,
        if col_both == 0 { 0.0 } else { col_diff as f64 / col_both as f64 }
    ));
    log(format!("So sánh lớp bề mặt: khớp tên khối {:.1}% trên {} khối (bỏ qua {} khối do quặng/đặc điểm)", pct(s_ok, s_tot), s_tot, s_feat));
    let mut wt: Vec<_> = water_top.into_iter().collect();
    wt.sort_by(|a, b| b.1.cmp(&a.1));
    let wt: Vec<String> = wt.iter().take(3).map(|(y, n)| format!("y={y}: {n} cột")).collect();
    log(format!("Mực nước thật (khối nước cao nhất): {}; bộ máy dùng nước y < {}", wt.join(", "), sea));
    let mut v: Vec<_> = s_conf.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for ((p, r), n) in v.iter().take(8) {
        log(format!("  lệch bề mặt: đoán {p} nhưng thật là {r} ({n} khối)"));
    }
    compare_biomes(&router, dir, chunks, log);
}

/// So quần xã đoán được với quần xã thật lưu trong chunk, theo từng ô 4x4x4.
fn compare_biomes(router: &Router, dir: &Path, chunks: &[(i32, i32)], log: &dyn Fn(String)) {
    let t0 = std::time::Instant::now();
    let src = BiomeSource::new();
    let (raws, _) = world::load_raw(dir, chunks, 1);
    let (mut tot, mut ok) = (0u64, 0u64);
    let mut conf: HashMap<(String, String), u64> = HashMap::new();
    for rc in &raws {
        for (sy, pal, idx) in &rc.biomes {
            for cy in 0..4usize {
                for cz in 0..4usize {
                    for cx in 0..4usize {
                        let k = match idx {
                            Some(v) => v[(cy * 4 + cz) * 4 + cx] as usize,
                            None => 0,
                        };
                        let Some(real) = pal.get(k) else { continue };
                        let real = real.strip_prefix("minecraft:").unwrap_or(real);
                        let pred = src.find(&router.climate(rc.cx * 4 + cx as i32, sy * 4 + cy as i32, rc.cz * 4 + cz as i32));
                        tot += 1;
                        if pred == real {
                            ok += 1;
                        } else {
                            *conf.entry((pred.to_string(), real.to_string())).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
    }
    log(format!("So sánh quần xã ({} mục bảng, {} ms): khớp {:.1}% trên {} ô", src.count(), t0.elapsed().as_millis(), pct(ok, tot), tot));
    let mut v: Vec<_> = conf.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for ((p, r), n) in v.iter().take(6) {
        log(format!("  lệch: đoán {p} nhưng thật là {r} ({n} ô)"));
    }
}
