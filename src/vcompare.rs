//! Công cụ so sánh: sinh địa hình bằng bộ máy hàm mật độ rồi đối chiếu với chunk thật của thế giới.
use crate::{
    assets::Jar,
    vdens::Router,
    world::{World, UNKNOWN},
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

fn pct(a: u64, b: u64) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 * 100.0 / b as f64
    }
}

pub fn compare(jar: &Jar, seed: i64, w: &World, keys: &[String], chunks: &[(i32, i32)], log: &dyn Fn(String)) {
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
    let (mut tot, mut ok, mut extra, mut missing) = (0u64, 0u64, 0u64, 0u64);
    let (mut cols, mut col_ok, mut col_diff, mut col_both) = (0u64, 0u64, 0i64, 0u64);
    for &(cx, cz) in chunks {
        let (x0, z0) = (cx * 16, cz * 16);
        router.prepare(x0, z0);
        for z in z0..z0 + 16 {
            for x in x0..x0 + 16 {
                let (mut rtop, mut ptop) = (i32::MIN, i32::MIN);
                for y in (-64..=250).rev() {
                    let id = w.get(x, y, z);
                    if id == UNKNOWN {
                        continue;
                    }
                    let pred = router.density(x, y, z) > 0.0;
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
}
