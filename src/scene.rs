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

/// "minecraft:oak_stairs[facing=east]" -> "oak_stairs"
pub fn base_name(k: &str) -> String {
    let k = k.strip_prefix("minecraft:").unwrap_or(k);
    k.split('[').next().unwrap_or(k).to_string()
}

pub struct SceneData {
    /// tên block theo id toàn cục lúc vào game
    pub names: Vec<String>,
    pub atlas: Vec<u8>,
    pub chunks: Vec<ChunkMesh>,
    /// vị trí mắt lúc vào game
    pub spawn: [f32; 3],
    pub stream: Option<Streamer>,
    /// dữ liệu va chạm ban đầu (rỗng với cảnh thử: bay tự do)
    pub shapes: Vec<Shape>,
    pub col: Vec<((i32, i32), Arc<Chunk>)>,
    pub items: crate::items::Items,
    /// kênh gửi vị trí lên server (chế độ nhiều người chơi)
    pub net: Option<mpsc::Sender<crate::netplay::NetOut>>,
    /// trạng thái từ server (chat, người chơi...)
    pub net_state: Option<crate::netplay::SharedState>,
}

/// Tin từ luồng stream gửi sang luồng vẽ.
pub enum Msg {
    Add(ChunkMesh),
    Drop(i32, i32),
    /// hình dạng va chạm của các block state mới (nối tiếp vào danh sách)
    Shapes(Vec<Shape>),
    /// tên block (không thuộc tính) của các id mới, cùng thứ tự với Shapes
    Names(Vec<String>),
    /// chunk mới nạp (dữ liệu khối) cho vật lý
    Col(i32, i32, Arc<Chunk>),
    ColDrop(i32, i32),
    /// server đặt lại vị trí (x, y chân, z, yaw và pitch theo độ kiểu Minecraft)
    Teleport([f64; 3], f32, f32),
    Kick(String),
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
    let items = crate::items::build(&jar, &atlas, log);
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
    Ok(SceneData { atlas: atlas.pixels, chunks: vec![ChunkMesh::new(mesh, 0, 0)], spawn: [7.0, 1.7, 4.0], stream: None, shapes: Vec::new(), col: Vec::new(), items, net: None, net_state: None, names: Vec::new() })
}

/// Độ sáng 0..1 từ ánh sáng ô (lấy mức cao hơn giữa sáng trời và sáng khối), kèm một chút sáng nền.
pub fn bright(l: u8) -> f32 {
    let x = (l >> 4).max(l & 15) as f32 / 15.0;
    0.03 + 0.97 * x / (4.0 - 3.0 * x)
}

/// Smooth lighting: mỗi đỉnh lấy trung bình ánh sáng của 4 ô quanh nó (lớp ô phía trước mặt) nhân với AO
/// (ô đặc làm tối như Minecraft). Trả về độ sáng 0..1 cho 4 đỉnh của mặt.
fn smooth(world: &World, states: &[State], p: [i32; 3], f: &crate::assets::BFace) -> [f32; 4] {
    let d = f.cull as usize;
    let a = [1usize, 1, 2, 2, 0, 0][d] ;
    let (t1, t2) = ((a + 1) % 3, (a + 2) % 3);
    let n = [p[0] + DV[d][0], p[1] + DV[d][1], p[2] + DV[d][2]];
    // lưới 3x3 các ô trong lớp phía trước: chỉ số (i+1, j+1) với i theo t1, j theo t2
    let mut opaque = [[false; 3]; 3];
    let mut light = [[0.0f32; 3]; 3];
    let center = bright(world.light(n[0], n[1], n[2]));
    for i in 0..3 {
        for j in 0..3 {
            let mut q = n;
            q[t1] += i as i32 - 1;
            q[t2] += j as i32 - 1;
            let id = world.get(q[0], q[1], q[2]);
            let op = id != UNKNOWN && id != 0 && states[id as usize].occl == 0x3f;
            opaque[i][j] = op;
            light[i][j] = if op { center } else { bright(world.light(q[0], q[1], q[2])) };
        }
    }
    let mut out = [0.0f32; 4];
    for k in 0..4 {
        let s1 = if f.q[k][t1] > 0.5 { 2 } else { 0 };
        let s2 = if f.q[k][t2] > 0.5 { 2 } else { 0 };
        let (oa, ob) = (opaque[s1][1], opaque[1][s2]);
        // ô góc: nếu cả hai ô bên cạnh đặc thì coi góc đặc luôn (ánh sáng không lọt qua khe chéo)
        let (ok, lk) = if oa && ob { (true, center) } else { (opaque[s1][s2], light[s1][s2]) };
        let l = (center + light[s1][1] + light[1][s2] + lk) / 4.0;
        let sh = |o: bool| if o { 0.2f32 } else { 1.0 };
        let ao = (1.0 + sh(oa) + sh(ob) + sh(ok)) / 4.0;
        out[k] = l * ao;
    }
    out
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
                        if st.fluid == 1 {
                            // nước: sáng phẳng theo ô phía trước mặt
                            let lv = if f.cull != NOCULL {
                                let d = f.cull as usize;
                                world.light(x + DV[d][0], y + DV[d][1], z + DV[d][2])
                            } else {
                                world.light(x, y, z)
                            };
                            mesh.push_water(f, at, col, bright(lv));
                        } else if f.cull != NOCULL {
                            let br = smooth(world, states, [x, y, z], f);
                            mesh.push_face_v(f, at, col, br);
                        } else {
                            let br = bright(world.light(x, y, z));
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
enum Src {
    Disk(PathBuf),
    Net(crate::netplay::Link),
}

struct Worker {
    src: Src,
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
        // sinh chunk bằng bộ máy worldgen ở chỗ thế giới chưa tạo (chỉ chế độ đọc đĩa)
        let seed = match &self.src {
            Src::Disk(d) => crate::vrng::read_seed(d),
            _ => None,
        };
        let mut gen_router: Option<crate::vdens::Router> = None;
        let mut gen_off = seed.is_none();
        let mut gen_tried: HashSet<(i32, i32)> = HashSet::new();
        let mut gen_count = 0usize;
        while !stop.load(Relaxed) {
            let (pcx, pcz) = unpack(pos.load(Relaxed));
            let far = |x: i32, z: i32| (x - pcx).abs() > r + 3 || (z - pcz).abs() > r + 3;
            let dist = |c: &(i32, i32)| (c.0 - pcx).pow(2) + (c.1 - pcz).pow(2);

            let net = matches!(self.src, Src::Net(_));
            // bỏ chunk quá xa (chế độ mạng: server tự báo hủy chunk)
            if !net {
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
            }

            // nạp chunk còn thiếu (thêm một vòng đệm để dựng mesh được vòng trong), gần trước
            let mut busy = false;
            let mut raws: Vec<RawChunk> = Vec::new();
            let mut want: Vec<(i32, i32)> = Vec::new();
            let mut edits: Vec<(i32, i32, i32, u32)> = Vec::new();
            match &self.src {
                Src::Disk(dir) => {
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
                    if !want.is_empty() {
                        busy = true;
                        raws = world::load_raw(dir, &want, threads).0;
                    }
                }
                Src::Net(link) => {
                    let mut n = 0;
                    while n < 24 {
                        match link.rx.try_recv() {
                            Ok(crate::netplay::NetIn::Chunk(rc)) => {
                                want.push((rc.cx, rc.cz));
                                raws.push(rc);
                                n += 1;
                            }
                            Ok(crate::netplay::NetIn::Unload(x, z)) => {
                                self.w.chunks.remove(&(x, z));
                                self.tried.remove(&(x, z));
                                self.meshed.remove(&(x, z));
                                if tx.send(Msg::ColDrop(x, z)).is_err() || tx.send(Msg::Drop(x, z)).is_err() {
                                    return;
                                }
                            }
                            Ok(crate::netplay::NetIn::Tp { x, y, z, yaw, pitch }) => {
                                if tx.send(Msg::Teleport([x, y, z], yaw, pitch)).is_err() {
                                    return;
                                }
                            }
                            Ok(crate::netplay::NetIn::Kick(m)) => {
                                let _ = tx.send(Msg::Kick(m));
                                return;
                            }
                            Ok(crate::netplay::NetIn::Joined) => {}
                            Ok(crate::netplay::NetIn::Block(x, y, z, s)) => {
                                edits.push((x, y, z, s));
                                n += 1;
                            }
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => return,
                        }
                    }
                    busy = n > 0;
                }
            }
            if let Some(sd) = seed {
                if !gen_off {
                    gen_tried.retain(|&(x, z)| !far(x, z));
                    let mut miss: Vec<(i32, i32)> = Vec::new();
                    for dz in -(r + 1)..=(r + 1) {
                        for dx in -(r + 1)..=(r + 1) {
                            let c = (pcx + dx, pcz + dz);
                            if self.tried.contains(&c) && !self.w.chunks.contains_key(&c) && !gen_tried.contains(&c) {
                                miss.push(c);
                            }
                        }
                    }
                    miss.sort_by_key(|c| dist(c));
                    miss.truncate(2);
                    if !miss.is_empty() && gen_router.is_none() {
                        match crate::vdens::Router::build(&jar, sd) {
                            Ok(rt) => gen_router = Some(rt),
                            Err(_) => gen_off = true,
                        }
                    }
                    if let Some(rt) = &gen_router {
                        for c in miss {
                            gen_tried.insert(c);
                            raws.push(crate::vgen::generate(rt, c.0, c.1));
                            want.push(c);
                            for d in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                                self.meshed.remove(&(c.0 + d.0, c.1 + d.1));
                            }
                            gen_count += 1;
                            if gen_count % 64 == 0 {
                                rt.clear_caches();
                            }
                            busy = true;
                        }
                    }
                }
            }
            if !edits.is_empty() {
                for (x, y, z, sid) in edits {
                    let key = crate::netdata::state_key(sid);
                    let id = match self.ids.get(&key) {
                        Some(&i) => i,
                        None => {
                            let i = self.keys.len() as u16;
                            self.ids.insert(key.clone(), i);
                            self.keys.push(key);
                            i
                        }
                    };
                    let before = self.states.len();
                    grow_states(&mut res, &self.keys, &mut self.states);
                    if self.states.len() > before {
                        let sh: Vec<Shape> = self.states[before..].iter().map(|s| s.shape()).collect();
                        let nm: Vec<String> = self.keys[before..].iter().map(|k| base_name(k)).collect();
                        let _ = tx.send(Msg::Names(nm));
                        let nm: Vec<String> = self.keys[before..].iter().map(|k| base_name(k)).collect();
                    let _ = tx.send(Msg::Names(nm));
                    if tx.send(Msg::Shapes(sh)).is_err() {
                            return;
                        }
                    }
                    if let Some(ch) = world::set_block(&mut self.w, x, y, z, id) {
                        let (cx, cz) = (x >> 4, z >> 4);
                        if tx.send(Msg::Col(cx, cz, ch)).is_err() {
                            return;
                        }
                        self.meshed.remove(&(cx, cz));
                        if x & 15 == 0 {
                            self.meshed.remove(&(cx - 1, cz));
                        }
                        if x & 15 == 15 {
                            self.meshed.remove(&(cx + 1, cz));
                        }
                        if z & 15 == 0 {
                            self.meshed.remove(&(cx, cz - 1));
                        }
                        if z & 15 == 15 {
                            self.meshed.remove(&(cx, cz + 1));
                        }
                        busy = true;
                    }
                }
            }
            if busy {
                for c in &want {
                    self.tried.insert(*c);
                }
                let added = ingest(&mut self.w, &mut self.ids, &mut self.keys, raws);
                let before = self.states.len();
                grow_states(&mut res, &self.keys, &mut self.states);
                if self.states.len() > before {
                    let sh: Vec<Shape> = self.states[before..].iter().map(|s| s.shape()).collect();
                    let nm: Vec<String> = self.keys[before..].iter().map(|k| base_name(k)).collect();
                    let _ = tx.send(Msg::Names(nm));
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
    crate::vrng::selftest(log);
    match crate::vrng::read_seed(dir) {
        Some(sd) => log(format!("Hạt giống thế giới: {sd}")),
        None => log("Không đọc được hạt giống thế giới từ level.dat".to_string()),
    }
    let items = crate::items::build(&jar, &atlas, log);

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
    if let Some(sd) = crate::vrng::read_seed(dir) {
        let cs: Vec<(i32, i32)> = [(ccx, ccz), (ccx + 1, ccz), (ccx, ccz + 1), (ccx + 1, ccz + 1)].into_iter().filter(|c| w.chunks.contains_key(c)).collect();
        crate::vcompare::compare(&jar, sd, dir, &w, &keys, &cs, log);
    }

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
    let worker = Worker { src: Src::Disk(dir.to_path_buf()), w, ids, keys, states, tried: list.iter().copied().collect(), meshed: coords.iter().copied().collect() };
    let pixels = atlas.pixels.clone();
    let (p2, s2) = (pos.clone(), stop.clone());
    std::thread::spawn(move || worker.run(jar, atlas, radius, p2, s2, tx));
    log(format!("Tổng thời gian dựng: {} ms", t0.elapsed().as_millis()));
    Ok(SceneData { atlas: pixels, chunks, spawn: [center[0] as f32, y, center[2] as f32], stream: Some(Streamer { rx, pos, stop }), shapes, col, items, net: None, net_state: None, names: Vec::new() })
}

/// Dựng cảnh từ server: chờ vị trí đầu tiên, rồi stream chunk nhận qua mạng.
pub fn build_net(jar_path: &Path, link: crate::netplay::Link, log: &dyn Fn(String)) -> Result<SceneData, String> {
    use crate::netplay::NetIn;
    let t0 = std::time::Instant::now();
    let jar = Jar::open(jar_path)?;
    let atlas = build_atlas(&jar, log)?;
    let items = crate::items::build_net(&jar, &atlas, log);
    log(format!("Tài nguyên xong sau {} ms, chờ server...", t0.elapsed().as_millis()));
    // chờ gói đặt vị trí đầu tiên; gom chunk đến trước
    let mut early: Vec<NetIn> = Vec::new();
    let mut spawn: Option<[f64; 3]> = None;
    let t1 = std::time::Instant::now();
    while spawn.is_none() {
        if t1.elapsed() > Duration::from_secs(45) {
            return Err("Timed out waiting for the server to send the player position".into());
        }
        match link.rx.recv_timeout(Duration::from_millis(100)) {
            Ok(NetIn::Tp { x, y, z, yaw, pitch }) => {
                spawn = Some([x, y, z]);
                early.push(NetIn::Tp { x, y, z, yaw, pitch });
            }
            Ok(NetIn::Kick(m)) => return Err(m),
            Ok(o) => early.push(o),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err("Connection lost".into()),
        }
    }
    let sp = spawn.unwrap();
    log(format!("Server đặt vị trí {:.1} {:.1} {:.1}", sp[0], sp[1], sp[2]));
    let (tx, rx) = mpsc::channel();
    let (pcx, pcz) = ((sp[0].floor() as i32) >> 4, (sp[2].floor() as i32) >> 4);
    let pos = Arc::new(AtomicI64::new(pack(pcx, pcz)));
    let stop = Arc::new(AtomicBool::new(false));
    let net_tx = link.tx.clone();
    let net_state = link.state.clone();
    if let Ok(b) = jar.read("assets/minecraft/lang/en_us.json") {
        if let Some(crate::json::Value::Obj(m)) = String::from_utf8(b).ok().and_then(|t| crate::json::parse(&t).ok()) {
            let l: HashMap<String, String> = m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect();
            let _ = crate::chat::LANG.set(l);
        }
    }
    // chunk/teleport đã nhận sớm: đẩy lại cho worker xử lý như bình thường (qua kênh nội bộ không cần; worker đọc link.rx)
    // nên đưa lại vào hàng bằng cách xử lý trực tiếp ở đây
    let mut ids: HashMap<String, u16> = HashMap::new();
    let mut keys: Vec<String> = vec!["minecraft:air".into()];
    ids.insert("minecraft:air".into(), 0);
    let mut w = World::default();
    let mut raws = Vec::new();
    let mut tried = HashSet::new();
    for m in early {
        if let NetIn::Chunk(rc) = m {
            tried.insert((rc.cx, rc.cz));
            raws.push(rc);
        }
    }
    let added = ingest(&mut w, &mut ids, &mut keys, raws);
    let mut states: Vec<State> = Vec::new();
    {
        let mut res = Resolver::new(&jar, &atlas);
        grow_states(&mut res, &keys, &mut states);
    }
    let shapes: Vec<Shape> = states.iter().map(|s| s.shape()).collect();
    let names: Vec<String> = keys.iter().map(|k| base_name(k)).collect();
    let col: Vec<((i32, i32), Arc<Chunk>)> = added.iter().map(|(x, z, c)| ((*x, *z), c.clone())).collect();
    let worker = Worker { src: Src::Net(link), w, ids, keys, states, tried, meshed: HashSet::new() };
    let pixels = atlas.pixels.clone();
    let (p2, s2) = (pos.clone(), stop.clone());
    std::thread::spawn(move || worker.run(jar, atlas, 32, p2, s2, tx));
    Ok(SceneData { atlas: pixels, chunks: Vec::new(), spawn: [sp[0] as f32, sp[1] as f32 + 1.62, sp[2] as f32], stream: Some(Streamer { rx, pos, stop }), shapes, col, items, net: Some(net_tx), net_state: Some(net_state), names })
}
