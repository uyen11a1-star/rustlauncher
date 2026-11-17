//! Đọc thế giới Minecraft Java: level.dat, file vùng .mca, chunk (NBT) -> mảng id block state.
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering::Relaxed},
        Mutex,
    },
};

use crate::{inflate, nbt::Tag};

/// Số section theo chiều cao, bắt đầu từ Y = -8 (đủ cho overworld -64..320 và dư).
pub const NSEC: usize = 48;
pub const SEC0: i32 = -8;
/// Trả về khi hỏi khối ở chunk chưa nạp.
pub const UNKNOWN: u16 = u16::MAX;

pub enum Sec {
    Uniform(u16),
    Full(Box<[u16; 4096]>),
}

pub struct Chunk {
    pub secs: Vec<Option<Sec>>,
}

#[derive(Default)]
pub struct World {
    pub chunks: HashMap<(i32, i32), Chunk>,
}

impl World {
    pub fn get(&self, x: i32, y: i32, z: i32) -> u16 {
        let Some(c) = self.chunks.get(&(x >> 4, z >> 4)) else { return UNKNOWN };
        let si = (y >> 4) - SEC0;
        if si < 0 || si >= NSEC as i32 {
            return 0;
        }
        match &c.secs[si as usize] {
            None => 0,
            Some(Sec::Uniform(id)) => *id,
            Some(Sec::Full(a)) => a[(((y & 15) << 8) | ((z & 15) << 4) | (x & 15)) as usize],
        }
    }
}

/// Chunk thô: palette dạng chuỗi + chỉ số cục bộ (chưa gán id toàn cục).
pub struct RawSection {
    pub y: i32,
    pub palette: Vec<String>,
    pub idx: Option<Vec<u16>>,
}

pub struct RawChunk {
    pub cx: i32,
    pub cz: i32,
    pub secs: Vec<RawSection>,
}

pub struct LevelInfo {
    pub spawn: Option<[f64; 3]>,
    pub from_player: bool,
    pub version: String,
}

pub fn read_level(dir: &Path) -> Result<LevelInfo, String> {
    let bytes = std::fs::read(dir.join("level.dat")).map_err(|e| format!("không đọc được level.dat: {e}"))?;
    let raw = inflate::gzip_any(&bytes, 64 << 20)?;
    let root = crate::nbt::parse(&raw)?;
    let data = root.get("Data").ok_or("level.dat không có Data")?;
    let version = data.get("Version").and_then(|v| v.get("Name")).and_then(|n| n.as_str()).unwrap_or("?").to_string();
    if let Some(pos) = data.get("Player").and_then(|p| p.get("Pos")).and_then(|p| p.as_list()) {
        if pos.len() == 3 {
            if let (Some(x), Some(y), Some(z)) = (pos[0].as_f64(), pos[1].as_f64(), pos[2].as_f64()) {
                return Ok(LevelInfo { spawn: Some([x, y, z]), from_player: true, version });
            }
        }
    }
    let int = |k: &str| data.get(k).and_then(|v| v.as_i64());
    if let (Some(x), Some(y), Some(z)) = (int("SpawnX"), int("SpawnY"), int("SpawnZ")) {
        return Ok(LevelInfo { spawn: Some([x as f64, y as f64, z as f64]), from_player: false, version });
    }
    if let Some(p) = data.get("spawn").and_then(|s| s.get("pos")).and_then(|p| p.as_ints()) {
        if p.len() == 3 {
            return Ok(LevelInfo { spawn: Some([p[0] as f64, p[1] as f64, p[2] as f64]), from_player: false, version });
        }
    }
    Ok(LevelInfo { spawn: None, from_player: false, version })
}

/// Tìm một vùng bất kỳ (r.X.Z.mca) để lấy tâm khi level.dat không có điểm sinh.
pub fn any_region_center(dir: &Path) -> Option<[f64; 3]> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join("region")).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let (mut sx, mut sz, mut n) = (0.0f64, 0.0f64, 0.0f64);
    for name in names {
        let p: Vec<&str> = name.split('.').collect();
        if p.len() == 4 && p[0] == "r" && p[3] == "mca" {
            if let (Ok(x), Ok(z)) = (p[1].parse::<i32>(), p[2].parse::<i32>()) {
                sx += x as f64 * 512.0 + 256.0;
                sz += z as f64 * 512.0 + 256.0;
                n += 1.0;
            }
        }
    }
    // trung bình tâm các vùng: với 4 vùng quanh gốc thì ra đúng (0, 0)
    if n > 0.0 { Some([sx / n, 100.0, sz / n]) } else { None }
}

fn read_chunk_bytes(dir: &Path, cx: i32, cz: i32) -> Result<Option<Vec<u8>>, String> {
    let path: PathBuf = dir.join("region").join(format!("r.{}.{}.mca", cx >> 5, cz >> 5));
    let mut f = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let idx = ((cx & 31) + (cz & 31) * 32) as u64;
    let mut ent = [0u8; 4];
    f.seek(SeekFrom::Start(idx * 4)).map_err(|e| e.to_string())?;
    f.read_exact(&mut ent).map_err(|e| format!("header vùng: {e}"))?;
    let sector = ((ent[0] as u64) << 16) | ((ent[1] as u64) << 8) | ent[2] as u64;
    if sector == 0 || ent[3] == 0 {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(sector * 4096)).map_err(|e| e.to_string())?;
    let mut h = [0u8; 5];
    f.read_exact(&mut h).map_err(|e| format!("đầu chunk: {e}"))?;
    let len = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize;
    if len < 1 || len > 16 << 20 {
        return Err(format!("độ dài chunk lạ: {len}"));
    }
    let mut body = vec![0u8; len - 1];
    f.read_exact(&mut body).map_err(|e| format!("thân chunk: {e}"))?;
    match h[4] {
        1 => inflate::gzip_any(&body, 64 << 20).map(Some),
        2 => inflate::zlib_any(&body, 64 << 20).map(Some),
        3 => Ok(Some(body)),
        4 => Err("chunk nén LZ4 chưa hỗ trợ".into()),
        c => Err(format!("kiểu nén chunk {c} chưa hỗ trợ")),
    }
}

fn prop_key(p: &Tag) -> Option<String> {
    let name = p.get("Name")?.as_str()?;
    let mut s = name.to_string();
    if let Some(Tag::Compound(m)) = p.get("Properties") {
        let mut kv: Vec<(&String, &Tag)> = m.iter().collect();
        kv.sort_by(|a, b| a.0.cmp(b.0));
        s.push('[');
        for (i, (k, v)) in kv.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(k);
            s.push('=');
            s.push_str(v.as_str().unwrap_or(""));
        }
        s.push(']');
    }
    Some(s)
}

fn parse_chunk(cx: i32, cz: i32, root: &Tag) -> Result<Option<RawChunk>, String> {
    if root.get("Level").is_some() {
        return Err("thế giới cũ (trước 1.18) chưa hỗ trợ".into());
    }
    if let Some(st) = root.get("Status").and_then(|s| s.as_str()) {
        if !st.ends_with("full") {
            return Ok(None);
        }
    }
    let secs = root.get("sections").and_then(|s| s.as_list()).ok_or("chunk không có sections")?;
    let mut out = Vec::new();
    for s in secs {
        let y = s.get("Y").and_then(|y| y.as_i64()).ok_or("section thiếu Y")? as i32;
        let Some(bs) = s.get("block_states") else { continue };
        let pal = bs.get("palette").and_then(|p| p.as_list()).ok_or("thiếu palette")?;
        let palette: Vec<String> = pal.iter().filter_map(prop_key).collect();
        if palette.is_empty() {
            continue;
        }
        let idx = if palette.len() == 1 {
            None
        } else {
            let data = bs.get("data").and_then(|d| d.as_longs()).ok_or("thiếu data của palette")?;
            let mut bits = 4usize;
            while (1usize << bits) < palette.len() {
                bits += 1;
            }
            let per = 64 / bits;
            if data.len() * per < 4096 {
                return Err("data palette ngắn".into());
            }
            let mask = (1u64 << bits) - 1;
            let mut v = Vec::with_capacity(4096);
            for i in 0..4096usize {
                let l = data[i / per] as u64;
                v.push(((l >> ((i % per) * bits)) & mask) as u16);
            }
            Some(v)
        };
        out.push(RawSection { y, palette, idx });
    }
    Ok(Some(RawChunk { cx, cz, secs: out }))
}

pub struct LoadStats {
    pub loaded: usize,
    pub empty: usize,
    pub errors: usize,
    pub first_error: Option<String>,
}

/// Đọc song song các chunk trong danh sách. Trả về các chunk thô.
pub fn load_raw(dir: &Path, list: &[(i32, i32)], threads: usize) -> (Vec<RawChunk>, LoadStats) {
    let next = AtomicUsize::new(0);
    let out = Mutex::new(Vec::new());
    let stats = Mutex::new(LoadStats { loaded: 0, empty: 0, errors: 0, first_error: None });
    std::thread::scope(|sc| {
        for _ in 0..threads.max(1) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Relaxed);
                if i >= list.len() {
                    break;
                }
                let (cx, cz) = list[i];
                let r = read_chunk_bytes(dir, cx, cz)
                    .and_then(|b| match b {
                        None => Ok(None),
                        Some(raw) => {
                            let root = crate::nbt::parse(&raw)?;
                            parse_chunk(cx, cz, &root)
                        }
                    });
                let mut st = stats.lock().unwrap();
                match r {
                    Ok(Some(c)) => {
                        st.loaded += 1;
                        drop(st);
                        out.lock().unwrap().push(c);
                    }
                    Ok(None) => st.empty += 1,
                    Err(e) => {
                        st.errors += 1;
                        if st.first_error.is_none() {
                            st.first_error = Some(format!("chunk {cx},{cz}: {e}"));
                        }
                    }
                }
            });
        }
    });
    (out.into_inner().unwrap(), stats.into_inner().unwrap())
}
