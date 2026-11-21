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
    mcgui::net::{put_str, put_varint, rd_str, rd_varint, recv, send, Conn},
    netdata,
    world::{RawChunk, RawSection},
};

pub enum NetIn {
    /// khối đổi: x, y, z, id block state mạng
    Block(i32, i32, i32, u32),
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
    /// tin nhắn; bắt đầu bằng '/' là lệnh
    Chat(String),
    /// chọn ô thanh nhanh 0..8
    Held(u8),
    /// đào: status 0 bắt đầu, 1 hủy, 2 xong; face 0..5
    Dig { status: i32, pos: [i32; 3], face: u8 },
    /// đặt khối / tương tác lên khối
    Place { pos: [i32; 3], face: u8, cur: [f32; 3] },
    /// dùng vật phẩm cầm trên tay (không nhắm khối)
    Use { yaw: f32, pitch: f32 },
    Swing,
    /// bấm ô `slot` của cửa sổ vật chứa `win` (click trái thường)
    Click { win: i32, state: i32, slot: i16, button: i8, mode: i32 },
    /// đóng cửa sổ vật chứa
    CloseWin(i32),
}

pub use crate::winnet::{click_packet, NetWin};

static SEQ: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

fn next_seq() -> i32 {
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

fn pack_pos(p: [i32; 3]) -> [u8; 8] {
    let v = (((p[0] as i64) & 0x3FFFFFF) << 38) | (((p[2] as i64) & 0x3FFFFFF) << 12) | ((p[1] as i64) & 0xFFF);
    v.to_be_bytes()
}

fn unpack_pos(v: i64) -> [i32; 3] {
    [(v >> 38) as i32, (v << 52 >> 52) as i32, (v << 26 >> 38) as i32]
}

fn rd_varlong(b: &[u8], i: &mut usize) -> Option<i64> {
    let mut v: u64 = 0;
    for sh in 0..10 {
        let x = *b.get(*i)?;
        *i += 1;
        v |= ((x & 0x7f) as u64) << (7 * sh);
        if x & 0x80 == 0 {
            return Some(v as i64);
        }
    }
    None
}

/// Người chơi khác thấy được trong thế giới.
#[derive(Clone)]
pub struct Pl {
    pub uuid: [u8; 16],
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head: f32,
}

/// Trạng thái nhận từ server, luồng đọc ghi vào, luồng vẽ lấy ra.
#[derive(Default)]
pub struct NetState {
    /// dòng chat mới chưa hiện (đã đổi sang mã §)
    pub chat: Vec<String>,
    /// uuid -> tên người chơi (từ player_info)
    pub names: std::collections::HashMap<[u8; 16], String>,
    /// túi đồ theo bố cục của ta (46 ô); (id vật phẩm mạng, số lượng)
    pub inv: Vec<Option<(u32, u8)>>,
    pub inv_dirty: bool,
    /// server đổi ô thanh nhanh đang chọn
    pub held: Option<u8>,
    pub players: std::collections::HashMap<i32, Pl>,
    /// số thứ tự hành động mới nhất server đã xác nhận (acknowledge_player_digging)
    pub ack: i32,
    /// cửa sổ vật chứa đang mở (None = không có)
    pub win: Option<NetWin>,
    /// luồng đọc đặt true khi `win` đổi (mở, đóng, nội dung); luồng vẽ đọc rồi xóa
    pub win_dirty: bool,
}

/// Ô của cửa sổ 0 (túi người chơi) -> ô của ta.
fn win0_slot(s: i32) -> Option<usize> {
    match s {
        0 => Some(45),
        1..=4 => Some(40 + s as usize),
        5..=8 => Some(36 + (s - 5) as usize),
        9..=35 => Some(s as usize),
        36..=44 => Some((s - 36) as usize),
        45 => Some(40),
        _ => None,
    }
}

/// Ô của gói set_player_inventory -> ô của ta.
fn inv_slot(s: i32) -> Option<usize> {
    match s {
        0..=35 => Some(s as usize),
        36 => Some(39),
        37 => Some(38),
        38 => Some(37),
        39 => Some(36),
        40 => Some(40),
        _ => None,
    }
}

fn set_inv(st: &SharedState, slot: Option<usize>, v: Option<(u32, u8)>) {
    let Some(i) = slot else { return };
    let mut g = st.lock().unwrap();
    if g.inv.len() < 46 {
        g.inv.resize(46, None);
    }
    g.inv[i] = v;
    g.inv_dirty = true;
}

/// Bỏ qua vector chuyển động nén (lpVec3).
fn skip_lpvec(b: &[u8], i: &mut usize) -> Option<()> {
    let b0 = *b.get(*i)?;
    *i += 1;
    if b0 == 0 {
        return Some(());
    }
    *i += 5;
    if b0 & 4 != 0 {
        rd_varint(b, i)?;
    }
    Some(())
}

fn rd_i16(b: &[u8], i: &mut usize) -> Option<i16> {
    let s = b.get(*i..*i + 2)?;
    *i += 2;
    Some(i16::from_be_bytes([s[0], s[1]]))
}

fn ang(b: &[u8], i: &mut usize) -> f32 {
    let v = b.get(*i).copied().unwrap_or(0) as i8;
    *i += 1;
    v as f32 * 360.0 / 256.0
}

/// Gói thực thể liên quan người chơi.
fn entity_packet(id: i32, b: &[u8], st: &SharedState) -> Option<()> {
    let mut i = 0;
    match id {
        0x01 => {
            let eid = rd_varint(b, &mut i)?;
            let uuid = uuid_at(b, &mut i)?;
            let ty = rd_varint(b, &mut i)?;
            if ty != 151 {
                return Some(());
            }
            let (x, y, z) = (rd_f64(b, &mut i)?, rd_f64(b, &mut i)?, rd_f64(b, &mut i)?);
            skip_lpvec(b, &mut i)?;
            let pitch = ang(b, &mut i);
            let yaw = ang(b, &mut i);
            let head = ang(b, &mut i);
            st.lock().unwrap().players.insert(eid, Pl { uuid, pos: [x, y, z], yaw, pitch, head: if head == 0.0 { yaw } else { head } });
        }
        0x4b => {
            let n = rd_varint(b, &mut i)?;
            let mut g = st.lock().unwrap();
            for _ in 0..n {
                let e = rd_varint(b, &mut i)?;
                g.players.remove(&e);
            }
        }
        0x33 | 0x34 | 0x36 => {
            let eid = rd_varint(b, &mut i)?;
            let mut d = [0.0f64; 3];
            if id != 0x36 {
                for k in 0..3 {
                    d[k] = rd_i16(b, &mut i)? as f64 / 4096.0;
                }
            }
            let look = if id != 0x33 { Some((ang(b, &mut i), ang(b, &mut i))) } else { None };
            let mut g = st.lock().unwrap();
            if let Some(p) = g.players.get_mut(&eid) {
                for k in 0..3 {
                    p.pos[k] += d[k];
                }
                if let Some((y, pi)) = look {
                    p.yaw = y;
                    p.pitch = pi;
                }
            }
        }
        0x7b | 0x23 => {
            let eid = rd_varint(b, &mut i)?;
            let (x, y, z) = (rd_f64(b, &mut i)?, rd_f64(b, &mut i)?, rd_f64(b, &mut i)?);
            i += 24;
            let (yaw, pitch) = (rd_f32(b, &mut i)?, rd_f32(b, &mut i)?);
            let flags = if id == 0x7b { rd_i32(b, &mut i)? as u32 } else { 0 };
            let mut g = st.lock().unwrap();
            if let Some(p) = g.players.get_mut(&eid) {
                let r = |bit: u32, new: f64, old: f64| if flags & (1 << bit) != 0 { old + new } else { new };
                p.pos = [r(0, x, p.pos[0]), r(1, y, p.pos[1]), r(2, z, p.pos[2])];
                p.yaw = r(3, yaw as f64, p.yaw as f64) as f32;
                p.pitch = r(4, pitch as f64, p.pitch as f64) as f32;
            }
        }
        0x51 => {
            let eid = rd_varint(b, &mut i)?;
            let h = ang(b, &mut i);
            if let Some(p) = st.lock().unwrap().players.get_mut(&eid) {
                p.head = h;
            }
        }
        _ => {}
    }
    Some(())
}

pub type SharedState = Arc<Mutex<NetState>>;

pub struct Link {
    pub rx: Receiver<NetIn>,
    pub tx: Sender<NetOut>,
    pub state: SharedState,
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
    /// false trong lúc cấu hình lại (server chuyển máy chủ): không được gửi gói của pha chơi
    play: bool,
}

type Shared = Arc<Mutex<Wr>>;

/// Gửi gói của pha chơi; bị bỏ qua (không lỗi) khi đang ở pha cấu hình lại.
fn wsend_play(w: &Shared, id: i32, body: &[u8]) -> Result<(), String> {
    let mut g = w.lock().unwrap();
    if !g.play {
        return Ok(());
    }
    let c = g.comp;
    send(&mut g.s, id, body, c)
}

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
    let wr: Shared = Arc::new(Mutex::new(Wr { s: conn.s, comp: conn.comp, play: true }));
    let (tx_in, rx_in) = mpsc::channel::<NetIn>();
    let (tx_out, rx_out) = mpsc::channel::<NetOut>();
    let cur: Arc<Mutex<[f64; 5]>> = Arc::new(Mutex::new([0.0; 5]));
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let state: SharedState = Default::default();
    let st2 = state.clone();

    // luồng ghi: gửi vị trí
    {
        let (wr, cur, alive) = (wr.clone(), cur.clone(), alive.clone());
        std::thread::spawn(move || {
            while let Ok(m) = rx_out.recv() {
                if !alive.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let (x, y, z, yaw, pitch, ground) = match m {
                    NetOut::Pos { x, y, z, yaw, pitch, ground } => (x, y, z, yaw, pitch, ground),
                    NetOut::Dig { status, pos, face } => {
                        let mut b = Vec::new();
                        put_varint(&mut b, status as u32);
                        b.extend_from_slice(&pack_pos(pos));
                        b.push(face);
                        put_varint(&mut b, next_seq() as u32);
                        if wsend_play(&wr, 0x28, &b).is_err() {
                            break;
                        }
                        continue;
                    }
                    NetOut::Place { pos, face, cur } => {
                        let mut b = Vec::new();
                        put_varint(&mut b, 0);
                        b.extend_from_slice(&pack_pos(pos));
                        put_varint(&mut b, face as u32);
                        for c in cur {
                            b.extend_from_slice(&c.to_be_bytes());
                        }
                        b.push(0);
                        b.push(0);
                        put_varint(&mut b, next_seq() as u32);
                        if wsend_play(&wr, 0x3f, &b).is_err() {
                            break;
                        }
                        continue;
                    }
                    NetOut::Use { yaw, pitch } => {
                        let mut b = Vec::new();
                        put_varint(&mut b, 0);
                        put_varint(&mut b, next_seq() as u32);
                        b.extend_from_slice(&yaw.to_be_bytes());
                        b.extend_from_slice(&pitch.to_be_bytes());
                        if wsend_play(&wr, 0x40, &b).is_err() {
                            break;
                        }
                        continue;
                    }
                    NetOut::Swing => {
                        if wsend_play(&wr, 0x3c, &[0]).is_err() {
                            break;
                        }
                        continue;
                    }
                    NetOut::Click { win, state, slot, button, mode } => {
                        if wsend_play(&wr, 0x11, &click_packet(win, state, slot, button, mode)).is_err() {
                            break;
                        }
                        continue;
                    }
                    NetOut::CloseWin(id) => {
                        let mut b = Vec::new();
                        put_varint(&mut b, id as u32);
                        if wsend_play(&wr, 0x12, &b).is_err() {
                            break;
                        }
                        continue;
                    }
                    NetOut::Held(h) => {
                        if wsend_play(&wr, 0x34, &(h as i16).to_be_bytes()).is_err() {
                            break;
                        }
                        continue;
                    }
                    NetOut::Chat(t) => {
                        let mut b = Vec::new();
                        if let Some(c) = t.strip_prefix('/') {
                            put_str(&mut b, c);
                            if wsend_play(&wr, 0x06, &b).is_err() {
                                break;
                            }
                        } else {
                            put_str(&mut b, &t);
                            let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
                            b.extend_from_slice(&ms.to_be_bytes());
                            b.extend_from_slice(&0i64.to_be_bytes());
                            b.push(0); // không chữ ký
                            put_varint(&mut b, 0);
                            b.extend_from_slice(&[0, 0, 0]);
                            b.push(0);
                            if wsend_play(&wr, 0x08, &b).is_err() {
                                break;
                            }
                        }
                        continue;
                    }
                };
                *cur.lock().unwrap() = [x, y, z, yaw as f64, pitch as f64];
                let mut b = Vec::with_capacity(34);
                for v in [x, y, z] {
                    b.extend_from_slice(&v.to_be_bytes());
                }
                b.extend_from_slice(&yaw.to_be_bytes());
                b.extend_from_slice(&pitch.to_be_bytes());
                b.push(ground as u8);
                if wsend_play(&wr, 0x1e, &b).is_err() {
                    break;
                }
            }
        });
    }

    // luồng đọc
    std::thread::spawn(move || {
        let mut s = rs;
        let comp = conn.comp;
        let res = reader(&mut s, comp, &wr, &tx_in, &cur, &st2);
        alive.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Err(e) = res {
            let _ = tx_in.send(NetIn::Kick(e));
        }
    });
    Ok(Link { rx: rx_in, tx: tx_out, state, guard })
}

fn reader(s: &mut TcpStream, comp: bool, wr: &Shared, tx: &Sender<NetIn>, cur: &Arc<Mutex<[f64; 5]>>, st: &SharedState) -> Result<(), String> {
    let mut ctx = Ctx { min_y: -64, height: 384, dims: Vec::new(), biome_bits: 6, keys: Default::default() };
    let mut loaded: std::collections::HashSet<(i32, i32)> = Default::default();
    let mut cur_dim = String::new();
    // một phiên = cấu hình + chơi; server (proxy) chuyển máy chủ thì quay lại đầu vòng
    'session: loop {
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
                cur_dim = rd_str(&b, &mut i).unwrap_or_default();
                if let Some(&(mn, h)) = ctx.dims.get(dim) {
                    ctx.min_y = mn;
                    ctx.height = h;
                }
                wr.lock().unwrap().play = true;
                wsend(wr, 0x0d, &client_info())?;
                let _ = tx.send(NetIn::Joined);
            }
            0x74 => {
                // server (proxy BungeeCord/Velocity) chuyển sang máy chủ khác: xác nhận rồi vào lại pha cấu hình.
                // Cờ play tắt và gói xác nhận gửi trong cùng một lần khóa để luồng ghi không chen gói chơi vào.
                {
                    let mut g = wr.lock().unwrap();
                    g.play = false;
                    let c = g.comp;
                    send(&mut g.s, 0x0f, &[], c)?;
                }
                for (x, z) in loaded.drain() {
                    let _ = tx.send(NetIn::Unload(x, z));
                }
                {
                    let mut g = st.lock().unwrap();
                    g.players.clear();
                    g.win = None;
                    g.win_dirty = true;
                }
                continue 'session;
            }
            0x50 => {
                // respawn: sang thế giới khác thì bỏ chunk cũ và đổi chiều cao thế giới
                let dim = rd_varint(&b, &mut i).unwrap_or(0) as usize;
                let name = rd_str(&b, &mut i).unwrap_or_default();
                if name != cur_dim {
                    cur_dim = name;
                    if let Some(&(mn, h)) = ctx.dims.get(dim) {
                        ctx.min_y = mn;
                        ctx.height = h;
                    }
                    for (x, z) in loaded.drain() {
                        let _ = tx.send(NetIn::Unload(x, z));
                    }
                    st.lock().unwrap().players.clear();
                }
            }
            0x7f => {
                let host = rd_str(&b, &mut i).unwrap_or_default();
                let port = rd_varint(&b, &mut i).unwrap_or(0);
                return Err(format!("Disconnected: server yêu cầu chuyển sang {host}:{port} (chưa hỗ trợ transfer)"));
            }
            0x2b => wsend(wr, 0x1b, &b)?,
            0x3b => wsend(wr, 0x2c, &b)?,
            0x20 => return Err(format!("Disconnected: {}", nbt_text(&b))),
            0x2c => match parse_chunk(&mut ctx, &b) {
                Some(rc) => {
                    loaded.insert((rc.cx, rc.cz));
                    let _ = tx.send(NetIn::Chunk(rc));
                }
                None => {}
            },
            0x25 => {
                let z = rd_i32(&b, &mut i).unwrap_or(0);
                let x = rd_i32(&b, &mut i).unwrap_or(0);
                loaded.remove(&(x, z));
                let _ = tx.send(NetIn::Unload(x, z));
            }
            0x0b => {
                wsend(wr, 0x0a, &4.0f32.to_be_bytes())?;
                batches += 1;
                if batches == 1 {
                    wsend(wr, 0x2b, &[])?;
                }
            }
            0x77 => {
                let mut j = 0;
                if let Some(t) = crate::chat::read_nbt(&b, &mut j) {
                    let action_bar = b.get(j).copied().unwrap_or(0) != 0;
                    if !action_bar {
                        st.lock().unwrap().chat.push(crate::chat::to_legacy(&t));
                    }
                }
            }
            0x21 => {
                let mut j = 0;
                if let Some(m) = crate::chat::read_nbt(&b, &mut j) {
                    let kind = rd_varint(&b, &mut j).unwrap_or(1).saturating_sub(1) as usize;
                    let name = crate::chat::read_nbt(&b, &mut j).map(|t| crate::chat::to_legacy(&t)).unwrap_or_default();
                    st.lock().unwrap().chat.push(crate::chat::typed(kind, &name, &crate::chat::to_legacy(&m)));
                }
            }
            0x3f => {
                if let Some(line) = parse_player_chat(&b, st) {
                    st.lock().unwrap().chat.push(line);
                }
            }
            0x08 => {
                if let Some(raw) = b.get(0..8) {
                    let p = unpack_pos(i64::from_be_bytes(raw.try_into().unwrap()));
                    i = 8;
                    if let Some(s) = rd_varint(&b, &mut i) {
                        let _ = tx.send(NetIn::Block(p[0], p[1], p[2], s as u32));
                    }
                }
            }
            0x52 => {
                if let Some(raw) = b.get(0..8) {
                    let v = i64::from_be_bytes(raw.try_into().unwrap());
                    let (sx, sz, sy) = ((v >> 42) as i32, (v << 22 >> 42) as i32, (v << 44 >> 44) as i32);
                    i = 8;
                    let n = rd_varint(&b, &mut i).unwrap_or(0);
                    for _ in 0..n {
                        let Some(r) = rd_varlong(&b, &mut i) else { break };
                        let (lx, lz, ly) = (((r >> 8) & 15) as i32, ((r >> 4) & 15) as i32, (r & 15) as i32);
                        let _ = tx.send(NetIn::Block(sx * 16 + lx, sy * 16 + ly, sz * 16 + lz, (r >> 12) as u32));
                    }
                }
            }
            0x04 => {
                if let Some(s) = rd_varint(&b, &mut i) {
                    st.lock().unwrap().ack = s;
                }
            }
            0x44 => parse_player_info(&b, st),
            0x01 | 0x4b | 0x33 | 0x34 | 0x36 | 0x7b | 0x23 | 0x51 => {
                let _ = entity_packet(id, &b, st);
            }
            0x39 | 0x11 => {
                let mut g = st.lock().unwrap();
                let g = &mut *g;
                crate::winnet::apply(&mut g.win, &mut g.win_dirty, id, &b);
            }
            0x12 => {
                let win = rd_varint(&b, &mut i).unwrap_or(-1);
                rd_varint(&b, &mut i);
                if win == 0 {
                    let n = rd_varint(&b, &mut i).unwrap_or(0);
                    let mut inv: Vec<Option<(u32, u8)>> = vec![None; 46];
                    let mut ok = true;
                    for k in 0..n {
                        match crate::slot::read_slot(&b, &mut i) {
                            Some(v) => {
                                if let Some(o) = win0_slot(k) {
                                    inv[o] = v;
                                }
                            }
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    let mut g = st.lock().unwrap();
                    if ok || g.inv.len() < 46 {
                        g.inv = inv;
                        g.inv_dirty = true;
                    }
                } else {
                    let mut g = st.lock().unwrap();
                    let g = &mut *g;
                    crate::winnet::apply(&mut g.win, &mut g.win_dirty, id, &b);
                }
            }
            0x14 => {
                let win = rd_varint(&b, &mut i).unwrap_or(-1);
                rd_varint(&b, &mut i);
                let slot = rd_i16(&b, &mut i).unwrap_or(-1) as i32;
                if win == 0 {
                    if let Some(v) = crate::slot::read_slot(&b, &mut i) {
                        set_inv(st, win0_slot(slot), v);
                    }
                } else {
                    let mut g = st.lock().unwrap();
                    let g = &mut *g;
                    crate::winnet::apply(&mut g.win, &mut g.win_dirty, id, &b);
                }
            }
            0x6a => {
                let slot = rd_varint(&b, &mut i).unwrap_or(-1);
                if let Some(v) = crate::slot::read_slot(&b, &mut i) {
                    set_inv(st, inv_slot(slot), v);
                }
            }
            0x67 => {
                let h = rd_varint(&b, &mut i).unwrap_or(0).clamp(0, 8) as u8;
                st.lock().unwrap().held = Some(h);
            }
            0x43 => {
                let n = rd_varint(&b, &mut i).unwrap_or(0);
                let mut g = st.lock().unwrap();
                for _ in 0..n {
                    if let Some(u) = b.get(i..i + 16) {
                        g.names.remove(<&[u8; 16]>::try_from(u).unwrap());
                    }
                    i += 16;
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
}

fn uuid_at(b: &[u8], i: &mut usize) -> Option<[u8; 16]> {
    let u: [u8; 16] = b.get(*i..*i + 16)?.try_into().ok()?;
    *i += 16;
    Some(u)
}

fn parse_player_chat(b: &[u8], st: &SharedState) -> Option<String> {
    let mut i = 0;
    rd_varint(b, &mut i)?;
    let uuid = uuid_at(b, &mut i)?;
    rd_varint(b, &mut i)?;
    if *b.get(i)? != 0 {
        i += 1 + 256;
    } else {
        i += 1;
    }
    let plain = rd_str(b, &mut i)?;
    i += 16;
    for _ in 0..rd_varint(b, &mut i)? {
        if rd_varint(b, &mut i)? == 0 {
            i += 256;
        }
    }
    let mut msg = plain;
    if *b.get(i)? != 0 {
        i += 1;
        let t = crate::chat::read_nbt(b, &mut i)?;
        msg = crate::chat::to_legacy(&t);
    } else {
        i += 1;
    }
    if rd_varint(b, &mut i)? == 2 {
        for _ in 0..rd_varint(b, &mut i)? {
            i += 8;
        }
    }
    let kind = rd_varint(b, &mut i)?.saturating_sub(1) as usize;
    let name = match crate::chat::read_nbt(b, &mut i) {
        Some(t) => crate::chat::to_legacy(&t),
        None => st.lock().unwrap().names.get(&uuid).cloned().unwrap_or_default(),
    };
    Some(crate::chat::typed(kind, &name, &msg))
}

fn parse_player_info(b: &[u8], st: &SharedState) {
    let mut i = 0;
    let Some(&act) = b.first() else { return };
    i += 1;
    let Some(n) = rd_varint(b, &mut i) else { return };
    for _ in 0..n {
        let Some(u) = uuid_at(b, &mut i) else { return };
        if act & 1 != 0 {
            let Some(name) = rd_str(b, &mut i) else { return };
            let Some(np) = rd_varint(b, &mut i) else { return };
            for _ in 0..np {
                if rd_str(b, &mut i).is_none() || rd_str(b, &mut i).is_none() {
                    return;
                }
                let Some(&has) = b.get(i) else { return };
                i += 1;
                if has != 0 && rd_str(b, &mut i).is_none() {
                    return;
                }
            }
            st.lock().unwrap().names.insert(u, name);
        }
        if act & 2 != 0 {
            let Some(&has) = b.get(i) else { return };
            i += 1;
            if has != 0 {
                i += 16 + 8;
                for _ in 0..2 {
                    let Some(l) = rd_varint(b, &mut i) else { return };
                    i += l as usize;
                }
            }
        }
        if act & 4 != 0 {
            rd_varint(b, &mut i);
        }
        if act & 8 != 0 {
            i += 1;
        }
        if act & 16 != 0 {
            rd_varint(b, &mut i);
        }
        if act & 32 != 0 {
            let Some(&has) = b.get(i) else { return };
            i += 1;
            if has != 0 && crate::chat::read_nbt(b, &mut i).is_none() {
                return;
            }
        }
        if act & 64 != 0 {
            rd_varint(b, &mut i);
        }
        if act & 128 != 0 {
            i += 1;
        }
    }
}

#[allow(dead_code)]
fn _unused(_: &mut dyn Read) {}
