//! Dựng cảnh 3D: cảnh thử (G1) và thế giới Java thật (G2).
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering::Relaxed},
        Mutex,
    },
};

use crate::{
    assets::{build_atlas, Jar, Mesh, NOCULL, V},
    blocks::{Resolver, State},
    world::{self, Chunk, Sec, World, NSEC, SEC0, UNKNOWN},
};

/// Mesh của một chunk (chỉ số cục bộ) kèm hộp bao [minx,miny,minz,maxx,maxy,maxz].
pub struct ChunkMesh {
    pub v: Vec<V>,
    pub i: Vec<u32>,
    pub bb: [f32; 6],
}

impl ChunkMesh {
    pub fn new(m: Mesh) -> ChunkMesh {
        let mut bb = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
        for v in &m.v {
            for k in 0..3 {
                bb[k] = bb[k].min(v.p[k]);
                bb[3 + k] = bb[3 + k].max(v.p[k]);
            }
        }
        ChunkMesh { v: m.v, i: m.i, bb }
    }
}

pub struct SceneData {
    pub atlas: Vec<u8>,
    pub chunks: Vec<ChunkMesh>,
    pub spawn: [f32; 3],
}

const BLOCKS: [&str; 32] = [
    "stone", "dirt", "grass_block", "oak_planks", "oak_log", "cobblestone", "sand", "gravel", "bricks", "bookshelf", "crafting_table", "furnace", "tnt", "diamond_ore", "gold_block", "iron_block", "glass", "oak_leaves", "oak_slab", "oak_stairs", "white_wool", "red_wool",
    "sandstone", "obsidian", "netherrack", "glowstone", "pumpkin", "melon", "bedrock", "coal_ore", "lapis_block", "emerald_block",
];

/// Hướng 0 down, 1 up, 2 north, 3 south, 4 west, 5 east.
const DV: [[i32; 3]; 6] = [[0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];

/// Cảnh thử: sàn cỏ + một dãy khối.
pub fn build_test(jar_path: &Path, log: &dyn Fn(String)) -> Result<SceneData, String> {
    let t0 = std::time::Instant::now();
    log(format!("Mở {}", jar_path.display()));
    let jar = Jar::open(jar_path)?;
    log(format!("jar có {} mục", jar.entries.len()));
    let atlas = build_atlas(&jar, log)?;
    let mut res = Resolver::new(&jar, &atlas);
    let mut mesh = Mesh::default();

    let floor = res.state("minecraft:grass_block");
    for x in -4..22 {
        for z in -14..8 {
            for f in &floor.faces {
                if f.cull != NOCULL {
                    let d = DV[f.cull as usize];
                    if d[1] == 0 && (-4..22).contains(&(x + d[0])) && (-14..8).contains(&(z + d[2])) {
                        continue; // mặt hông chạm khối sàn khác
                    }
                }
                mesh.push_face(f, [x as f32, -1.0, z as f32], if f.tint { floor.tint } else { [255; 3] });
            }
        }
    }
    let mut shown = 0;
    for (i, name) in BLOCKS.iter().enumerate() {
        let st = res.state(&format!("minecraft:{name}"));
        if st.faces.is_empty() {
            log(format!("khối {name}: không có mặt nào vẽ được"));
            continue;
        }
        let at = [((i % 8) * 2) as f32, 0.0, -((i / 8) as f32 * 2.0) - 2.0];
        for f in &st.faces {
            mesh.push_face(f, at, if f.tint { st.tint } else { [255; 3] });
        }
        shown += 1;
    }
    for m in res.missing.iter().take(10) {
        log(m.clone());
    }
    log(format!("Dựng {} khối, {} đỉnh, mất {} ms", shown, mesh.v.len(), t0.elapsed().as_millis()));
    Ok(SceneData { atlas: atlas.pixels, chunks: vec![ChunkMesh::new(mesh)], spawn: [7.0, 1.7, 4.0] })
}

fn mesh_chunk(world: &World, states: &[State], cx: i32, cz: i32) -> Mesh {
    let mut mesh = Mesh::default();
    let Some(chunk) = world.chunks.get(&(cx, cz)) else { return mesh };
    for (si, sec) in chunk.secs.iter().enumerate() {
        let Some(sec) = sec else { continue };
        let y0 = (si as i32 + SEC0) * 16;
        for ly in 0..16i32 {
            for lz in 0..16i32 {
                for lx in 0..16i32 {
                    let id = match sec {
                        Sec::Uniform(id) => *id,
                        Sec::Full(a) => a[((ly << 8) | (lz << 4) | lx) as usize],
                    };
                    if id == 0 {
                        continue;
                    }
                    let st = &states[id as usize];
                    if st.air || st.faces.is_empty() {
                        continue;
                    }
                    let (x, y, z) = (cx * 16 + lx, y0 + ly, cz * 16 + lz);
                    let nb = |d: usize| -> u16 {
                        let (dx, dy, dz) = (DV[d][0], DV[d][1], DV[d][2]);
                        let (nx, ny, nz) = (lx + dx, ly + dy, lz + dz);
                        if (0..16).contains(&nx) && (0..16).contains(&ny) && (0..16).contains(&nz) {
                            match sec {
                                Sec::Uniform(id) => *id,
                                Sec::Full(a) => a[((ny << 8) | (nz << 4) | nx) as usize],
                            }
                        } else {
                            world.get(x + dx, y + dy, z + dz)
                        }
                    };
                    for f in &st.faces {
                        if f.cull != NOCULL {
                            let n = nb(f.cull as usize);
                            if n == UNKNOWN {
                                continue;
                            }
                            let ns = &states[n as usize];
                            if ns.occl & (1 << (f.cull ^ 1)) != 0 {
                                continue;
                            }
                            if st.fluid != 0 && ns.fluid == st.fluid {
                                continue;
                            }
                        }
                        mesh.push_face(f, [x as f32, y as f32, z as f32], if f.tint { st.tint } else { [255; 3] });
                    }
                }
            }
        }
    }
    mesh
}

/// Dựng thế giới Java thật quanh điểm người chơi/điểm sinh, `radius` chunk mỗi hướng.
pub fn build_world(jar_path: &Path, dir: &Path, radius: i32, log: &dyn Fn(String)) -> Result<SceneData, String> {
    let t0 = std::time::Instant::now();
    let jar = Jar::open(jar_path)?;
    let atlas = build_atlas(&jar, log)?;
    log(format!("Atlas xong sau {} ms", t0.elapsed().as_millis()));

    let level = world::read_level(dir);
    let (spawn, from_player) = match &level {
        Ok(l) => {
            log(format!("level.dat: phiên bản {}", l.version));
            (l.spawn, l.from_player)
        }
        Err(e) => {
            log(format!("level.dat: {e}"));
            (None, false)
        }
    };
    let center = match spawn.or_else(|| world::any_region_center(dir)) {
        Some(c) => c,
        None => return Err("không thấy điểm sinh lẫn file vùng (thiếu thư mục region?)".into()),
    };
    let (ccx, ccz) = ((center[0].floor() as i32) >> 4, (center[2].floor() as i32) >> 4);
    log(format!("Tâm tại x {:.0} z {:.0} (chunk {ccx},{ccz}), bán kính {radius} chunk", center[0], center[2]));

    let mut list: Vec<(i32, i32)> = Vec::new();
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            list.push((ccx + dx, ccz + dz));
        }
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(4);
    let t1 = std::time::Instant::now();
    let (raws, stats) = world::load_raw(dir, &list, threads);
    log(format!("Chunk: nạp {}, rỗng/chưa tạo {}, lỗi {} ({} ms)", stats.loaded, stats.empty, stats.errors, t1.elapsed().as_millis()));
    if let Some(e) = &stats.first_error {
        log(format!("lỗi đầu tiên: {e}"));
    }
    if raws.is_empty() {
        return Err("không nạp được chunk nào".into());
    }

    // gán id toàn cục cho block state
    let mut ids: HashMap<String, u16> = HashMap::new();
    let mut keys: Vec<String> = Vec::new();
    ids.insert("minecraft:air".into(), 0);
    keys.push("minecraft:air".into());
    let mut w = World::default();
    for rc in raws {
        let mut secs: Vec<Option<Sec>> = (0..NSEC).map(|_| None).collect();
        for s in rc.secs {
            let si = s.y - SEC0;
            if si < 0 || si >= NSEC as i32 {
                continue;
            }
            let map: Vec<u16> = s
                .palette
                .iter()
                .map(|k| {
                    if let Some(&i) = ids.get(k) {
                        i
                    } else {
                        let i = keys.len() as u16;
                        ids.insert(k.clone(), i);
                        keys.push(k.clone());
                        i
                    }
                })
                .collect();
            secs[si as usize] = match s.idx {
                None => {
                    if map[0] == 0 {
                        None
                    } else {
                        Some(Sec::Uniform(map[0]))
                    }
                }
                Some(idx) => {
                    let mut a = Box::new([0u16; 4096]);
                    for (i, &p) in idx.iter().enumerate() {
                        a[i] = map.get(p as usize).copied().unwrap_or(0);
                    }
                    if a.iter().all(|&v| v == a[0]) {
                        if a[0] == 0 { None } else { Some(Sec::Uniform(a[0])) }
                    } else {
                        Some(Sec::Full(a))
                    }
                }
            };
        }
        w.chunks.insert((rc.cx, rc.cz), Chunk { secs });
    }
    log(format!("{} loại block state khác nhau", keys.len()));

    let t2 = std::time::Instant::now();
    let mut res = Resolver::new(&jar, &atlas);
    let states: Vec<State> = keys.iter().map(|k| res.state(k)).collect();
    log(format!("Đã dịch block state sang model ({} ms)", t2.elapsed().as_millis()));
    let mut seen = std::collections::HashSet::new();
    let mut shown = 0;
    for m in &res.missing {
        if seen.insert(m.clone()) {
            shown += 1;
            if shown <= 12 {
                log(format!("thiếu: {m}"));
            }
        }
    }
    if shown > 12 {
        log(format!("... và {} mục thiếu khác", shown - 12));
    }

    // dựng mesh song song theo chunk
    let t3 = std::time::Instant::now();
    let coords: Vec<(i32, i32)> = w.chunks.keys().copied().collect();
    let next = AtomicUsize::new(0);
    let parts: Mutex<Vec<ChunkMesh>> = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Relaxed);
                    if i >= coords.len() {
                        break;
                    }
                    let m = mesh_chunk(&w, &states, coords[i].0, coords[i].1);
                    if !m.v.is_empty() {
                        parts.lock().unwrap().push(ChunkMesh::new(m));
                    }
                }
            });
        }
    });
    let chunks = parts.into_inner().unwrap();
    let (nv, ni) = chunks.iter().fold((0, 0), |a, c| (a.0 + c.v.len(), a.1 + c.i.len()));
    log(format!("Dựng mesh: {} chunk, {} đỉnh, {} tam giác ({} ms)", chunks.len(), nv, ni / 3, t3.elapsed().as_millis()));

    // vị trí camera
    let (sx, sz) = (center[0].floor() as i32, center[2].floor() as i32);
    let y = if from_player {
        center[1] as f32 + 1.62
    } else {
        let mut found = None;
        for yy in (-64..320).rev() {
            let id = w.get(sx, yy, sz);
            if id == UNKNOWN {
                break;
            }
            if id != 0 && !states[id as usize].air {
                found = Some(yy);
                break;
            }
        }
        found.map_or(100.0, |v| v as f32 + 3.0)
    };
    log(format!("Tổng thời gian dựng: {} ms", t0.elapsed().as_millis()));
    Ok(SceneData { atlas: atlas.pixels, chunks, spawn: [center[0] as f32, y, center[2] as f32] })
}
