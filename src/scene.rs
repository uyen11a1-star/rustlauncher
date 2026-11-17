//! Dựng cảnh 3D: cảnh thử (G1) và thế giới Java thật (G2).
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering::Relaxed},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

use crate::{
    assets::{build_atlas, Atlas, Jar, Mesh, NOCULL, V},
    blocks::{Resolver, Shape, State},
    world::{self, Chunk, RawChunk, Sec, World, NSEC, SEC0, UNKNOWN},
};

/// Mesh của một chunk (chỉ số cục bộ) kèm hộp bao [minx,miny,minz,maxx,maxy,maxz].
pub struct ChunkMesh {
    pub cx: i32,
    pub cz: i32,
    pub v: Vec<V>,
    pub i: Vec<u32>,
    pub iw: Vec<u32>,
    pub bb: [f32; 6],
}

impl ChunkMesh {
    pub fn new(m: Mesh, cx: i32, cz: i32) -> ChunkMesh {
        let mut bb = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
        for v in &m.v {
            for k in 0..3 {
                bb[k] = bb[k].min(v.p[k]);
                bb[3 + k] = bb[3 + k].max(v.p[k]);
            }
        }
        ChunkMesh { cx, cz, v: m.v, i: m.i, iw: m.iw, bb }
    }
}

pub struct SceneData {
    pub atlas: Vec<u8>,
    pub chunks: Vec<ChunkMesh>,
    /// vị trí mắt lúc vào game
    pub spawn: [f32; 3],
    pub stream: Option<Streamer>,
    /// dữ liệu va chạm ban đầu (rỗng với cảnh thử: bay tự do)
    pub shapes: Vec<Shape>,
    pub col: Vec<((i32, i32), Arc<Chunk>)>,
}

/// Tin từ luồng stream gửi sang luồng vẽ.
pub enum Msg {
    Add(ChunkMesh),
    Drop(i32, i32),
    /// hình dạng va chạm của các block state mới (nối tiếp vào danh sách)
    Shapes(Vec<Shape>),
    /// chunk mới nạp (dữ liệu khối) cho vật lý
    Col(i32, i32, Arc<Chunk>),
    ColDrop(i32, i32),
}

/// Đầu nhận của luồng nền nạp/dựng chunk theo vị trí người chơi. Thả đi thì luồng nền tự dừng.
pub struct Streamer {
    pub rx: mpsc::Receiver<Msg>,
    pub pos: Arc<AtomicI64>,
    stop: Arc<AtomicBool>,
}

impl Drop for Streamer {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
    }
}

/// Gói tọa độ chunk của người chơi vào một số để chia sẻ giữa hai luồng.
pub fn pack(cx: i32, cz: i32) -> i64 {
    ((cx as i64) << 32) | (cz as u32 as i64)
}

fn unpack(v: i64) -> (i32, i32) {
    ((v >> 32) as i32, v as i32)
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
    Ok(SceneData { atlas: atlas.pixels, chunks: vec![ChunkMesh::new(mesh, 0, 0)], spawn: [7.0, 1.7, 4.0], stream: None, shapes: Vec::new(), col: Vec::new() })
}

/// Độ sáng 0..1 từ ánh sáng ô (lấy mức cao hơn giữa sáng trời và sáng khối), kèm một chút sáng nền.
fn bright(l: u8) -> f32 {
    let x = (l >> 4).max(l & 15) as f32 / 15.0;
    0.03 + 0.97 * x / (4.0 - 3.0 * x)
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
                        let at = [x as f32, y as f32, z as f32];
                        let col = if f.tint { st.tint } else { [255; 3] };
                        let lv = if f.cull != NOCULL {
                            let d = f.cull as usize;
                            world.light(x + DV[d][0], y + DV[d][1], z + DV[d][2])
                        } else {
                            world.light(x, y, z)
                        };
                        let br = bright(lv);
                        if st.fluid == 1 {
                            mesh.push_water(f, at, col, br);
                        } else {
                            mesh.push_face_b(f, at, col, br);
                        }
                    }
                }
            }
        }
    }
    mesh
}

/// Số chunk (mỗi hướng) dựng sẵn trước khi vào game; phần còn lại stream ở luồng nền.
const INIT_R: i32 = 2;

/// Gán id toàn cục cho block state của các chunk thô rồi đưa vào thế giới.
fn ingest(w: &mut World, ids: &mut HashMap<String, u16>, keys: &mut Vec<String>, raws: Vec<RawChunk>) -> Vec<(i32, i32, Arc<Chunk>)> {
    let mut added = Vec::new();
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
        // ánh sáng: SkyLight thiếu thì lấy lớp đáy của section phía trên (trên cùng = 15), BlockLight thiếu = 0
        let mut sky: Vec<Option<Vec<u8>>> = vec![None; NSEC];
        let mut blk: Vec<Option<Vec<u8>>> = vec![None; NSEC];
        for (y, s, b) in rc.lights {
            let si = y - SEC0;
            if si >= 0 && si < NSEC as i32 {
                sky[si as usize] = s;
                blk[si as usize] = b;
            }
        }
        let nib = |d: &Vec<u8>, i: usize| -> u8 {
            let v = d[i >> 1];
            if i & 1 == 0 { v & 15 } else { v >> 4 }
        };
        let mut lt: Vec<Option<Box<[u8; 4096]>>> = (0..NSEC).map(|_| None).collect();
        let mut above = [15u8; 256];
        for si in (0..NSEC).rev() {
            let mut a = Box::new([0u8; 4096]);
            let mut all = true;
            for i in 0..4096usize {
                let sk = match &sky[si] {
                    Some(d) => nib(d, i),
                    None => above[i & 255],
                };
                let bl = match &blk[si] {
                    Some(d) => nib(d, i),
                    None => 0,
                };
                a[i] = (sk << 4) | bl;
                if a[i] != 0xF0 {
                    all = false;
                }
            }
            for j in 0..256 {
                above[j] = a[j] >> 4;
            }
            if !all {
                lt[si] = Some(a);
            }
        }
        let ch = Arc::new(Chunk { secs, lt });
        w.chunks.insert((rc.cx, rc.cz), ch.clone());
        added.push((rc.cx, rc.cz, ch));
    }
    added
}

/// Dịch thêm các block state mới (từ `keys`) sang model.
fn grow_states(res: &mut Resolver, keys: &[String], states: &mut Vec<State>) {
    while states.len() < keys.len() {
        let st = res.state(&keys[states.len()]);
        states.push(st);
    }
}

/// Trạng thái của luồng stream: thế giới đang nạp, id block, các chunk đã thử đọc / đã dựng mesh.
struct Worker {
    dir: PathBuf,
    w: World,
    ids: HashMap<String, u16>,
    keys: Vec<String>,
    states: Vec<State>,
    tried: HashSet<(i32, i32)>,
    meshed: HashSet<(i32, i32)>,
}

impl Worker {
    fn run(mut self, jar: Jar, atlas: Atlas, r: i32, pos: Arc<AtomicI64>, stop: Arc<AtomicBool>, tx: mpsc::Sender<Msg>) {
        let mut res = Resolver::new(&jar, &atlas);
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(4);
        while !stop.load(Relaxed) {
            let (pcx, pcz) = unpack(pos.load(Relaxed));
            let far = |x: i32, z: i32| (x - pcx).abs() > r + 3 || (z - pcz).abs() > r + 3;
            let dist = |c: &(i32, i32)| (c.0 - pcx).pow(2) + (c.1 - pcz).pow(2);

            // bỏ chunk quá xa
            let gone: Vec<(i32, i32)> = self.w.chunks.keys().copied().filter(|&(x, z)| far(x, z)).collect();
            for k in gone {
                self.w.chunks.remove(&k);
                if tx.send(Msg::ColDrop(k.0, k.1)).is_err() {
                    return;
                }
            }
            self.tried.retain(|&(x, z)| !far(x, z));
            let dropped: Vec<(i32, i32)> = self.meshed.iter().copied().filter(|&(x, z)| far(x, z)).collect();
            for k in dropped {
                self.meshed.remove(&k);
                if tx.send(Msg::Drop(k.0, k.1)).is_err() {
                    return;
                }
            }

            // nạp chunk còn thiếu (thêm một vòng đệm để dựng mesh được vòng trong), gần trước
            let mut want: Vec<(i32, i32)> = Vec::new();
            for dz in -(r + 1)..=(r + 1) {
                for dx in -(r + 1)..=(r + 1) {
                    let c = (pcx + dx, pcz + dz);
                    if !self.tried.contains(&c) {
                        want.push(c);
                    }
                }
            }
            want.sort_by_key(|c| dist(c));
            want.truncate(6);
            let mut busy = !want.is_empty();
            if busy {
                let (raws, _) = world::load_raw(&self.dir, &want, threads);
                for c in &want {
                    self.tried.insert(*c);
                }
                let added = ingest(&mut self.w, &mut self.ids, &mut self.keys, raws);
                let before = self.states.len();
                grow_states(&mut res, &self.keys, &mut self.states);
                if self.states.len() > before {
                    let sh: Vec<Shape> = self.states[before..].iter().map(|s| s.shape()).collect();
                    if tx.send(Msg::Shapes(sh)).is_err() {
                        return;
                    }
                }
                for (x, z, ch) in added {
                    if tx.send(Msg::Col(x, z, ch)).is_err() {
                        return;
                    }
                }
            }

            // dựng mesh chunk nào đã đủ 4 chunk kề (đã thử đọc), gần trước
            let mut cands: Vec<(i32, i32)> = self
                .w
                .chunks
                .keys()
                .copied()
                .filter(|&(x, z)| {
                    (x - pcx).abs() <= r
                        && (z - pcz).abs() <= r
                        && !self.meshed.contains(&(x, z))
                        && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|d| self.tried.contains(&(x + d.0, z + d.1)))
                })
                .collect();
            cands.sort_by_key(|c| dist(c));
            for &(x, z) in cands.iter().take(4) {
                let m = mesh_chunk(&self.w, &self.states, x, z);
                self.meshed.insert((x, z));
                if !m.v.is_empty() && tx.send(Msg::Add(ChunkMesh::new(m, x, z))).is_err() {
                    return;
                }
                busy = true;
            }
            if !busy {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// Dựng thế giới Java thật quanh điểm người chơi/điểm sinh: dựng sẵn vùng nhỏ, rồi stream chunk theo `radius` ở luồng nền.
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
    log(format!("Tâm tại x {:.0} z {:.0} (chunk {ccx},{ccz}), tầm stream {radius} chunk", center[0], center[2]));

    // vùng đầu: INIT_R + 1 vòng đệm (vòng đệm chỉ để dựng mesh vòng trong cho đúng)
    let mut list: Vec<(i32, i32)> = Vec::new();
    for dz in -(INIT_R + 1)..=(INIT_R + 1) {
        for dx in -(INIT_R + 1)..=(INIT_R + 1) {
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

    let mut ids: HashMap<String, u16> = HashMap::new();
    let mut keys: Vec<String> = vec!["minecraft:air".into()];
    ids.insert("minecraft:air".into(), 0);
    let mut w = World::default();
    let _ = ingest(&mut w, &mut ids, &mut keys, raws);
    log(format!("{} loại block state khác nhau", keys.len()));

    let t2 = std::time::Instant::now();
    let mut states: Vec<State> = Vec::new();
    {
        let mut res = Resolver::new(&jar, &atlas);
        grow_states(&mut res, &keys, &mut states);
        log(format!("Đã dịch block state sang model ({} ms)", t2.elapsed().as_millis()));
        let mut seen = HashSet::new();
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
    }

    // dựng mesh vùng đầu song song theo chunk
    let t3 = std::time::Instant::now();
    let coords: Vec<(i32, i32)> = w.chunks.keys().copied().filter(|&(x, z)| (x - ccx).abs() <= INIT_R && (z - ccz).abs() <= INIT_R).collect();
    let next = AtomicUsize::new(0);
    let parts: Mutex<Vec<ChunkMesh>> = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Relaxed);
                if i >= coords.len() {
                    break;
                }
                let (x, z) = coords[i];
                let m = mesh_chunk(&w, &states, x, z);
                if !m.v.is_empty() {
                    parts.lock().unwrap().push(ChunkMesh::new(m, x, z));
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

    let shapes: Vec<Shape> = states.iter().map(|s| s.shape()).collect();
    let col: Vec<((i32, i32), Arc<Chunk>)> = w.chunks.iter().map(|(k, v)| (*k, v.clone())).collect();

    // giao thế giới cho luồng stream
    let pos = Arc::new(AtomicI64::new(pack(ccx, ccz)));
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let worker = Worker { dir: dir.to_path_buf(), w, ids, keys, states, tried: list.iter().copied().collect(), meshed: coords.iter().copied().collect() };
    let pixels = atlas.pixels.clone();
    let (p2, s2) = (pos.clone(), stop.clone());
    std::thread::spawn(move || worker.run(jar, atlas, radius, p2, s2, tx));
    log(format!("Tổng thời gian dựng: {} ms", t0.elapsed().as_millis()));
    Ok(SceneData { atlas: pixels, chunks, spawn: [center[0] as f32, y, center[2] as f32], stream: Some(Streamer { rx, pos, stop }), shapes, col })
}
