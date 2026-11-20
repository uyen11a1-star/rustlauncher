//! Phiên chơi trên server Minecraft Java 1.21.9/1.21.10 (giao thức 773): cấu hình, nhận chunk, đồng bộ vị trí.
use std::{
    io::Read,
    net::TcpStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    time::Duration,
};

use crate::{
    mcgui::net::{put_varint, rd_str, rd_varint, recv, send, Conn},
    netdata,
    world::{RawChunk, RawSection},
};

pub enum NetIn {
    Chunk(RawChunk),
    Unload(i32, i32),
    /// server đặt lại vị trí người chơi (tuyệt đối, đã áp cờ tương đối)
    Tp { x: f64, y: f64, z: f64, yaw: f32, pitch: f32 },
    /// vào được thế giới (đã nhận gói Login)
    Joined,
    Kick(String),
}

pub enum NetOut {
    Pos { x: f64, y: f64, z: f64, yaw: f32, pitch: f32, ground: bool },
}

pub struct Link {
    pub rx: Receiver<NetIn>,
    pub tx: Sender<NetOut>,
    guard: TcpStream,
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.guard.shutdown(std::net::Shutdown::Both);
    }
}

struct Wr {
    s: TcpStream,
    comp: bool,
}

type Shared = Arc<Mutex<Wr>>;

fn wsend(w: &Shared, id: i32, body: &[u8]) -> Result<(), String> {
    let mut g = w.lock().unwrap();
    let c = g.comp;
    send(&mut g.s, id, body, c)
}

fn client_info() -> Vec<u8> {
    let mut b = Vec::new();
    put_varint(&mut b, 5);
    b.extend_from_slice(b"en_us");
    b.push(8); // tầm nhìn (chunk)
    put_varint(&mut b, 0); // chat đầy đủ
    b.push(1); // màu chat
    b.push(0x7f); // các phần skin
    put_varint(&mut b, 1); // tay chính: phải
    b.push(0); // lọc văn bản
    b.push(1); // cho phép hiện trong danh sách
    put_varint(&mut b, 0); // hạt: tất cả
    b
}

/// Đọc NBT mạng (không tên gốc) và bỏ qua; có thể lấy các số nguyên ở mức gốc nếu `want` khác rỗng.
struct NbtR<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> NbtR<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.i..self.i + n)?;
        self.i += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<usize> {
        let s = self.take(2)?;
        Some(u16::from_be_bytes([s[0], s[1]]) as usize)
    }
    fn i32(&mut self) -> Option<i32> {
        let s = self.take(4)?;
        Some(i32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn string(&mut self) -> Option<String> {
        let n = self.u16()?;
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    /// bỏ qua payload của một tag; ghi lại số nguyên/chuỗi mức gốc vào `out`
    fn payload(&mut self, t: u8, depth: usize, out: Option<&mut Vec<(String, i64)>>, key: &str, text: Option<&mut String>) -> Option<()> {
        if depth > 32 {
            return None;
        }
        match t {
            1 => {
                self.take(1)?;
            }
            2 => {
                self.take(2)?;
            }
            3 => {
                let v = self.i32()?;
                if let Some(o) = out {
                    o.push((key.to_string(), v as i64));
                }
            }
            4 | 6 => {
                self.take(8)?;
            }
            5 => {
                self.take(4)?;
            }
            7 => {
                let n = self.i32()?.max(0) as usize;
                self.take(n)?;
            }
            8 => {
                let s = self.string()?;
                if let Some(t) = text {
                    if key == "text" || key.is_empty() {
                        t.push_str(&s);
                    }
                }
            }
            9 => {
                let et = *self.take(1)?.first()?;
                let n = self.i32()?.max(0) as usize;
                for _ in 0..n {
                    self.payload(et, depth + 1, None, "", None)?;
                }
            }
            10 => loop {
                let tt = *self.take(1)?.first()?;
                if tt == 0 {
                    break;
                }
                let k = self.string()?;
                self.payload(tt, depth + 1, None, &k, None)?;
            },
            11 => {
                let n = self.i32()?.max(0) as usize;
                self.take(n * 4)?;
            }
            12 => {
                let n = self.i32()?.max(0) as usize;
                self.take(n * 8)?;
            }
            _ => return None,
        }
        Some(())
    }
    /// NBT gốc không tên. Trả về các cặp (khoá, số nguyên) ở mức gốc, và chuỗi "text" nếu có.
    fn root(&mut self) -> Option<(Vec<(String, i64)>, String)> {
        let t = *self.take(1)?.first()?;
        let mut ints = Vec::new();
        let mut text = String::new();
        match t {
            0 => {}
            10 => loop {
                let tt = *self.take(1)?.first()?;
                if tt == 0 {
                    break;
                }
                let k = self.string()?;
                self.payload(tt, 1, Some(&mut ints), &k, Some(&mut text))?;
            },
            8 => {
                self.payload(8, 0, None, "", Some(&mut text))?;
            }
            _ => {
                self.payload(t, 0, None, "", None)?;
            }
        }
        Some((ints, text))
    }
}

fn nbt_text(b: &[u8]) -> String {
    let mut r = NbtR { b, i: 0 };
    r.root().map(|x| x.1).unwrap_or_default()
}

struct Ctx {
    min_y: i32,
    height: i32,
    /// thứ tự các dimension_type trong registry: (min_y, height)
    dims: Vec<(i32, i32)>,
    biome_bits: u32,
    keys: std::collections::HashMap<u32, String>,
}

fn rd_i32(b: &[u8], i: &mut usize) -> Option<i32> {
    let s = b.get(*i..*i + 4)?;
    *i += 4;
    Some(i32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn rd_i64(b: &[u8], i: &mut usize) -> Option<i64> {
    let s = b.get(*i..*i + 8)?;
    *i += 8;
    Some(i64::from_be_bytes(s.try_into().ok()?))
}

fn rd_f64(b: &[u8], i: &mut usize) -> Option<f64> {
    Some(f64::from_bits(rd_i64(b, i)? as u64))
}

fn rd_f32(b: &[u8], i: &mut usize) -> Option<f32> {
    Some(f32::from_bits(rd_i32(b, i)? as u32))
}

/// Container có palette: trả về (palette, chỉ số) hoặc giá trị đơn.
struct Cont {
    palette: Vec<u32>,
    /// None = một giá trị duy nhất (palette[0])
    idx: Option<Vec<u16>>,
    direct: bool,
}

fn read_container(d: &[u8], i: &mut usize, entries: usize, max_indirect: u8, direct_bits: u8) -> Option<Cont> {
    let bits = *d.get(*i)?;
    *i += 1;
    if bits == 0 {
        let v = rd_varint(d, i)? as u32;
        return Some(Cont { palette: vec![v], idx: None, direct: false });
    }
    let direct = bits > max_indirect;
    let mut palette = Vec::new();
    let bpe;
    if direct {
        bpe = direct_bits as usize;
    } else {
        let n = rd_varint(d, i)? as usize;
        if n > 4096 {
            return None;
        }
        for _ in 0..n {
            palette.push(rd_varint(d, i)? as u32);
        }
        bpe = bits as usize;
    }
    let per = 64 / bpe;
    let longs = entries.div_ceil(per);
    let raw = d.get(*i..*i + longs * 8)?;
    *i += longs * 8;
    let mask = (1u64 << bpe) - 1;
    let mut idx = Vec::with_capacity(entries);
    for e in 0..entries {
        let l = u64::from_be_bytes(raw[(e / per) * 8..(e / per) * 8 + 8].try_into().ok()?);
        idx.push(((l >> ((e % per) * bpe)) & mask) as u16);
    }
    Some(Cont { palette, idx: Some(idx), direct })
}

fn parse_chunk(ctx: &mut Ctx, b: &[u8]) -> Option<RawChunk> {
    let mut i = 0;
    let cx = rd_i32(b, &mut i)?;
    let cz = rd_i32(b, &mut i)?;
    // heightmaps
    for _ in 0..rd_varint(b, &mut i)? {
        rd_varint(b, &mut i)?;
        let n = rd_varint(b, &mut i)? as usize;
        i += n * 8;
    }
    let dn = rd_varint(b, &mut i)? as usize;
    let data = b.get(i..i + dn)?;
    i += dn;
    // block entities
    for _ in 0..rd_varint(b, &mut i)? {
        i += 3; // x/z, y
        rd_varint(b, &mut i)?;
        let mut r = NbtR { b, i };
        r.root()?;
        i = r.i;
    }
    let nsec = (ctx.height / 16).max(1) as usize;
    let min_sec = ctx.min_y >> 4;
    let mut secs = Vec::new();
    let mut di = 0;
    for s in 0..nsec {
        di += 2; // số block không phải khí
        let blocks = read_container(data, &mut di, 4096, 8, 15)?;
        read_container(data, &mut di, 64, 3, ctx.biome_bits.max(1) as u8)?;
        let y = min_sec + s as i32;
        let key = |ctx: &mut Ctx, id: u32| -> String { ctx.keys.entry(id).or_insert_with(|| netdata::state_key(id)).clone() };
        match blocks.idx {
            None => {
                let k = key(ctx, blocks.palette[0]);
                secs.push(RawSection { y, palette: vec![k], idx: None });
            }
            Some(idx) => {
                if blocks.direct {
                    let mut uniq: Vec<u32> = Vec::new();
                    let mut map: std::collections::HashMap<u32, u16> = std::collections::HashMap::new();
                    let mut out = Vec::with_capacity(4096);
                    for v in idx {
                        let g = v as u32;
                        let l = *map.entry(g).or_insert_with(|| {
                            uniq.push(g);
                            (uniq.len() - 1) as u16
                        });
                        out.push(l);
                    }
                    let palette = uniq.iter().map(|&g| key(ctx, g)).collect();
                    secs.push(RawSection { y, palette, idx: Some(out) });
                } else {
                    let palette = blocks.palette.iter().map(|&g| key(ctx, g)).collect();
                    secs.push(RawSection { y, palette, idx: Some(idx) });
                }
            }
        }
    }
    // ánh sáng: 4 mặt nạ rồi mảng 2048 byte cho mỗi bit đặt
    let mut masks: Vec<Vec<u64>> = Vec::new();
    for _ in 0..4 {
        let n = rd_varint(b, &mut i)? as usize;
        let mut m = Vec::with_capacity(n);
        for _ in 0..n {
            m.push(rd_i64(b, &mut i)? as u64);
        }
        masks.push(m);
    }
    let bit = |m: &Vec<u64>, k: usize| m.get(k / 64).map_or(false, |w| (w >> (k % 64)) & 1 == 1);
    let mut read_lights = |mask: &Vec<u64>, i: &mut usize| -> Option<Vec<Option<Vec<u8>>>> {
        let n = rd_varint(b, i)? as usize;
        let mut arrs = Vec::new();
        for _ in 0..n {
            let l = rd_varint(b, i)? as usize;
            arrs.push(b.get(*i..*i + l)?.to_vec());
            *i += l;
        }
        let total = nsec + 2;
        let mut out: Vec<Option<Vec<u8>>> = vec![None; total];
        let mut it = arrs.into_iter();
        for k in 0..total {
            if bit(mask, k) {
                out[k] = it.next();
            }
        }
        Some(out)
    };
    let sky = read_lights(&masks[0], &mut i)?;
    let blk = read_lights(&masks[1], &mut i)?;
    let mut lights = Vec::new();
    for k in 0..nsec + 2 {
        let y = min_sec - 1 + k as i32;
        let ok = |o: &Option<Vec<u8>>| o.as_ref().filter(|v| v.len() == 2048).cloned();
        // mặt nạ "rỗng" = toàn 0
        let s = ok(&sky[k]).or_else(|| if bit(&masks[2], k) { Some(vec![0; 2048]) } else { None });
        let bl = ok(&blk[k]).or_else(|| if bit(&masks[3], k) { Some(vec![0; 2048]) } else { None });
        lights.push((y, s, bl));
    }
    Some(RawChunk { cx, cz, secs, lights, biomes: Vec::new() })
}

/// Bắt đầu phiên: chạy luồng đọc + luồng ghi. Kết quả đến qua `Link::rx`.
pub fn start(conn: Conn) -> Result<Link, String> {
    let rs = conn.s.try_clone().map_err(|e| e.to_string())?;
    let guard = conn.s.try_clone().map_err(|e| e.to_string())?;
    let wr: Shared = Arc::new(Mutex::new(Wr { s: conn.s, comp: conn.comp }));
    let (tx_in, rx_in) = mpsc::channel::<NetIn>();
    let (tx_out, rx_out) = mpsc::channel::<NetOut>();
    let cur: Arc<Mutex<[f64; 5]>> = Arc::new(Mutex::new([0.0; 5]));
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));

    // luồng ghi: gửi vị trí
    {
        let (wr, cur, alive) = (wr.clone(), cur.clone(), alive.clone());
        std::thread::spawn(move || {
            while let Ok(m) = rx_out.recv() {
                if !alive.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let NetOut::Pos { x, y, z, yaw, pitch, ground } = m;
                *cur.lock().unwrap() = [x, y, z, yaw as f64, pitch as f64];
                let mut b = Vec::with_capacity(34);
                for v in [x, y, z] {
                    b.extend_from_slice(&v.to_be_bytes());
                }
                b.extend_from_slice(&yaw.to_be_bytes());
                b.extend_from_slice(&pitch.to_be_bytes());
                b.push(ground as u8);
                if wsend(&wr, 0x1e, &b).is_err() {
                    break;
                }
            }
        });
    }

    // luồng đọc
    std::thread::spawn(move || {
        let mut s = rs;
        let comp = conn.comp;
        let res = reader(&mut s, comp, &wr, &tx_in, &cur);
        alive.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Err(e) = res {
            let _ = tx_in.send(NetIn::Kick(e));
        }
    });
    Ok(Link { rx: rx_in, tx: tx_out, guard })
}

fn reader(s: &mut TcpStream, comp: bool, wr: &Shared, tx: &Sender<NetIn>, cur: &Arc<Mutex<[f64; 5]>>) -> Result<(), String> {
    let mut ctx = Ctx { min_y: -64, height: 384, dims: Vec::new(), biome_bits: 6, keys: Default::default() };
    // ---- giai đoạn cấu hình ----
    wsend(wr, 0x00, &client_info())?;
    loop {
        let (id, body) = recv(s, comp)?;
        let mut i = 0;
        match id {
            0x02 => return Err(format!("Disconnected: {}", nbt_text(&body))),
            0x03 => {
                wsend(wr, 0x03, &[])?;
                break;
            }
            0x04 => wsend(wr, 0x04, &body)?,
            0x05 => wsend(wr, 0x05, &body)?,
            0x0e => wsend(wr, 0x07, &[0])?,
            0x07 => {
                let reg = rd_str(&body, &mut i).unwrap_or_default();
                let n = rd_varint(&body, &mut i).unwrap_or(0);
                let mut dims = Vec::new();
                let mut count = 0;
                for _ in 0..n {
                    if rd_str(&body, &mut i).is_none() {
                        break;
                    }
                    let has = body.get(i).copied().unwrap_or(0);
                    i += 1;
                    count += 1;
                    if has == 1 {
                        let mut r = NbtR { b: &body, i };
                        let Some((ints, _)) = r.root() else { break };
                        i = r.i;
                        if reg == "minecraft:dimension_type" {
                            let g = |k: &str, d: i64| ints.iter().find(|x| x.0 == k).map_or(d, |x| x.1) as i32;
                            dims.push((g("min_y", -64), g("height", 384)));
                        }
                    } else if reg == "minecraft:dimension_type" {
                        dims.push((-64, 384));
                    }
                }
                if reg == "minecraft:dimension_type" && !dims.is_empty() {
                    ctx.dims = dims;
                }
                if reg == "minecraft:worldgen/biome" && count > 1 {
                    ctx.biome_bits = 32 - ((count - 1) as u32).leading_zeros();
                }
            }
            _ => {}
        }
    }
    // ---- giai đoạn chơi ----
    let mut batches = 0;
    loop {
        let (id, b) = recv(s, comp)?;
        let mut i = 0;
        match id {
            0x30 => {
                let _eid = rd_i32(&b, &mut i);
                i += 1;
                for _ in 0..rd_varint(&b, &mut i).unwrap_or(0) {
                    rd_str(&b, &mut i);
                }
                rd_varint(&b, &mut i);
                rd_varint(&b, &mut i);
                rd_varint(&b, &mut i);
                i += 3;
                let dim = rd_varint(&b, &mut i).unwrap_or(0) as usize;
                if let Some(&(mn, h)) = ctx.dims.get(dim) {
                    ctx.min_y = mn;
                    ctx.height = h;
                }
                wsend(wr, 0x0d, &client_info())?;
                let _ = tx.send(NetIn::Joined);
            }
            0x2b => wsend(wr, 0x1b, &b)?,
            0x3b => wsend(wr, 0x2c, &b)?,
            0x20 => return Err(format!("Disconnected: {}", nbt_text(&b))),
            0x2c => match parse_chunk(&mut ctx, &b) {
                Some(rc) => {
                    let _ = tx.send(NetIn::Chunk(rc));
                }
                None => {}
            },
            0x25 => {
                let z = rd_i32(&b, &mut i).unwrap_or(0);
                let x = rd_i32(&b, &mut i).unwrap_or(0);
                let _ = tx.send(NetIn::Unload(x, z));
            }
            0x0b => {
                wsend(wr, 0x0a, &20.0f32.to_be_bytes())?;
                batches += 1;
                if batches == 1 {
                    wsend(wr, 0x2b, &[])?;
                }
            }
            0x46 => {
                let tid = rd_varint(&b, &mut i).unwrap_or(0);
                let (x, y, z) = (rd_f64(&b, &mut i).unwrap_or(0.0), rd_f64(&b, &mut i).unwrap_or(0.0), rd_f64(&b, &mut i).unwrap_or(0.0));
                i += 24; // vận tốc
                let (yaw, pitch) = (rd_f32(&b, &mut i).unwrap_or(0.0), rd_f32(&b, &mut i).unwrap_or(0.0));
                let flags = rd_i32(&b, &mut i).unwrap_or(0) as u32;
                let c = *cur.lock().unwrap();
                let rel = |bit: u32, new: f64, old: f64| if flags & (1 << bit) != 0 { old + new } else { new };
                let (nx, ny, nz) = (rel(0, x, c[0]), rel(1, y, c[1]), rel(2, z, c[2]));
                let (nyaw, npitch) = (rel(3, yaw as f64, c[3]) as f32, rel(4, pitch as f64, c[4]) as f32);
                let mut ack = Vec::new();
                put_varint(&mut ack, tid as u32);
                wsend(wr, 0x00, &ack)?;
                let mut p = Vec::with_capacity(33);
                for v in [nx, ny, nz] {
                    p.extend_from_slice(&v.to_be_bytes());
                }
                p.extend_from_slice(&nyaw.to_be_bytes());
                p.extend_from_slice(&npitch.to_be_bytes());
                p.push(0);
                wsend(wr, 0x1e, &p)?;
                *cur.lock().unwrap() = [nx, ny, nz, nyaw as f64, npitch as f64];
                let _ = tx.send(NetIn::Tp { x: nx, y: ny, z: nz, yaw: nyaw, pitch: npitch });
            }
            _ => {}
        }
    }
}

#[allow(dead_code)]
fn _unused(_: &mut dyn Read) {}
