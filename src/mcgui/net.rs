//! Giao thức mạng Minecraft Java (TCP): ping trạng thái cho danh sách server, và bắt tay + đăng nhập thật.
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use crate::{inflate, json, json::Value, png};

/// Số giao thức của 1.21.10 (đổi khi hỗ trợ phiên bản khác).
pub const PROTOCOL: i32 = 773;

#[derive(Clone, Default)]
pub struct Status {
    /// 0 đang ping, 1 xong, 2 lỗi
    pub state: u8,
    pub motd: Vec<String>,
    pub online: i64,
    pub max: i64,
    pub version: String,
    pub protocol: i64,
    pub ping_ms: u32,
    pub icon: Option<png::Image>,
    pub icon_tex: Option<usize>,
    pub error: String,
    pub sample: Vec<String>,
}

pub fn split_addr(a: &str) -> (String, u16) {
    let a = a.trim();
    if let Some(r) = a.strip_prefix('[') {
        if let Some((h, rest)) = r.split_once(']') {
            return (h.to_string(), rest.trim_start_matches(':').parse().unwrap_or(25565));
        }
    }
    match a.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().unwrap_or(25565)),
        _ => (a.to_string(), 25565),
    }
}

fn connect(addr: &str, secs: u64) -> Result<TcpStream, String> {
    let (host, port) = split_addr(addr);
    let sa = (host.as_str(), port).to_socket_addrs().map_err(|e| format!("Unknown host: {host} ({e})"))?.next().ok_or("Unknown host")?;
    let s = TcpStream::connect_timeout(&sa, Duration::from_secs(secs)).map_err(|e| format!("Can't connect to server: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(secs))).ok();
    s.set_write_timeout(Some(Duration::from_secs(secs))).ok();
    s.set_nodelay(true).ok();
    Ok(s)
}

pub fn put_varint(out: &mut Vec<u8>, mut v: u32) {
    loop {
        if v < 0x80 {
            out.push(v as u8);
            return;
        }
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
}

pub fn put_str(out: &mut Vec<u8>, s: &str) {
    put_varint(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn read_varint(r: &mut dyn Read) -> Result<i32, String> {
    let (mut v, mut sh) = (0u32, 0);
    loop {
        let mut b = [0u8];
        r.read_exact(&mut b).map_err(|e| format!("Connection lost: {e}"))?;
        v |= ((b[0] & 0x7f) as u32) << sh;
        if b[0] & 0x80 == 0 {
            return Ok(v as i32);
        }
        sh += 7;
        if sh > 35 {
            return Err("Bad VarInt".into());
        }
    }
}

pub fn send(s: &mut TcpStream, id: i32, body: &[u8], compress: bool) -> Result<(), String> {
    let mut pkt = Vec::new();
    put_varint(&mut pkt, id as u32);
    pkt.extend_from_slice(body);
    let mut out = Vec::new();
    if compress {
        // dưới ngưỡng thì gửi không nén: Data Length = 0
        let mut inner = vec![0u8];
        inner.extend_from_slice(&pkt);
        put_varint(&mut out, inner.len() as u32);
        out.extend_from_slice(&inner);
    } else {
        put_varint(&mut out, pkt.len() as u32);
        out.extend_from_slice(&pkt);
    }
    s.write_all(&out).map_err(|e| format!("Connection lost: {e}"))
}

/// Đọc một gói; trả về (id, phần thân).
pub fn recv(s: &mut TcpStream, compress: bool) -> Result<(i32, Vec<u8>), String> {
    let len = read_varint(s)? as usize;
    if len == 0 || len > 8 << 20 {
        return Err("Bad packet length".into());
    }
    let mut buf = vec![0u8; len];
    s.read_exact(&mut buf).map_err(|e| format!("Connection lost: {e}"))?;
    let mut data = buf;
    if compress {
        let mut c = &data[..];
        let dl = read_varint(&mut c)? as usize;
        let rest = c.to_vec();
        data = if dl == 0 { rest } else { inflate::zlib_any(&rest, dl.max(1) + 16).map_err(|e| format!("Bad compressed packet: {e}"))? };
    }
    let mut c = &data[..];
    let id = read_varint(&mut c)?;
    Ok((id, c.to_vec()))
}

pub fn rd_varint(b: &[u8], i: &mut usize) -> Option<i32> {
    let (mut v, mut sh) = (0u32, 0);
    loop {
        let x = *b.get(*i)?;
        *i += 1;
        v |= ((x & 0x7f) as u32) << sh;
        if x & 0x80 == 0 {
            return Some(v as i32);
        }
        sh += 7;
        if sh > 35 {
            return None;
        }
    }
}

pub fn rd_str(b: &[u8], i: &mut usize) -> Option<String> {
    let n = rd_varint(b, i)? as usize;
    let s = b.get(*i..*i + n)?;
    *i += n;
    Some(String::from_utf8_lossy(s).into_owned())
}

fn handshake(s: &mut TcpStream, addr: &str, next: i32) -> Result<(), String> {
    let (host, port) = split_addr(addr);
    let mut b = Vec::new();
    put_varint(&mut b, PROTOCOL as u32);
    put_str(&mut b, &host);
    b.extend_from_slice(&port.to_be_bytes());
    put_varint(&mut b, next as u32);
    send(s, 0, &b, false)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let Some(v) = B64.iter().position(|&x| x == c) else { continue };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// Đổi thành phần chữ (JSON text component) sang chuỗi có mã màu §.
pub fn component_to_legacy(v: &Value) -> String {
    fn code(name: &str) -> Option<char> {
        Some(match name {
            "black" => '0',
            "dark_blue" => '1',
            "dark_green" => '2',
            "dark_aqua" => '3',
            "dark_red" => '4',
            "dark_purple" => '5',
            "gold" => '6',
            "gray" => '7',
            "dark_gray" => '8',
            "blue" => '9',
            "green" => 'a',
            "aqua" => 'b',
            "red" => 'c',
            "light_purple" => 'd',
            "yellow" => 'e',
            "white" => 'f',
            _ => return None,
        })
    }
    fn hex_near(h: &str) -> Option<char> {
        let h = h.strip_prefix('#')?;
        let n = u32::from_str_radix(h, 16).ok()?;
        let (r, g, b) = ((n >> 16) as i32, ((n >> 8) & 255) as i32, (n & 255) as i32);
        let pal: [(char, i32, i32, i32); 16] = [
            ('0', 0, 0, 0), ('1', 0, 0, 170), ('2', 0, 170, 0), ('3', 0, 170, 170), ('4', 170, 0, 0), ('5', 170, 0, 170), ('6', 255, 170, 0), ('7', 170, 170, 170),
            ('8', 85, 85, 85), ('9', 85, 85, 255), ('a', 85, 255, 85), ('b', 85, 255, 255), ('c', 255, 85, 85), ('d', 255, 85, 255), ('e', 255, 255, 85), ('f', 255, 255, 255),
        ];
        pal.iter().min_by_key(|p| (p.1 - r).pow(2) + (p.2 - g).pow(2) + (p.3 - b).pow(2)).map(|p| p.0)
    }
    fn go(v: &Value, out: &mut String, depth: usize) {
        if depth > 8 {
            return;
        }
        match v {
            Value::Str(s) => out.push_str(s),
            Value::Arr(a) => {
                for x in a {
                    go(x, out, depth + 1);
                }
            }
            Value::Obj(_) => {
                let col = v.get("color").and_then(|c| c.as_str()).and_then(|c| code(c).or_else(|| hex_near(c)));
                if let Some(c) = col {
                    out.push('§');
                    out.push(c);
                }
                if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
                    out.push_str(t);
                }
                if let Some(e) = v.get("extra") {
                    go(e, out, depth + 1);
                }
                if col.is_some() {
                    out.push_str("§r");
                }
            }
            _ => {}
        }
    }
    let mut s = String::new();
    go(v, &mut s, 0);
    s
}

/// Ping trạng thái: MOTD, số người, phiên bản, icon, độ trễ.
pub fn ping(addr: &str) -> Status {
    let mut st = Status::default();
    match ping_inner(addr, &mut st) {
        Ok(()) => st.state = 1,
        Err(e) => {
            st.state = 2;
            st.error = e;
        }
    }
    st
}

fn ping_inner(addr: &str, st: &mut Status) -> Result<(), String> {
    let mut s = connect(addr, 5)?;
    handshake(&mut s, addr, 1)?;
    send(&mut s, 0, &[], false)?;
    let (id, body) = recv(&mut s, false)?;
    if id != 0 {
        return Err("Unexpected status reply".into());
    }
    let mut i = 0;
    let text = rd_str(&body, &mut i).ok_or("Bad status reply")?;
    let v = json::parse(&text).map_err(|e| format!("Bad status JSON: {e}"))?;
    if let Some(d) = v.get("description") {
        let t = component_to_legacy(d);
        st.motd = t.split('\n').take(2).map(|x| x.to_string()).collect();
    }
    if let Some(p) = v.get("players") {
        st.online = p.get("online").and_then(|x| x.as_f64()).unwrap_or(0.0) as i64;
        st.max = p.get("max").and_then(|x| x.as_f64()).unwrap_or(0.0) as i64;
        if let Some(a) = p.get("sample").and_then(|x| x.as_arr()) {
            st.sample = a.iter().filter_map(|e| e.get("name").and_then(|n| n.as_str()).map(|n| n.to_string())).collect();
        }
    }
    if let Some(ver) = v.get("version") {
        st.version = ver.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        st.protocol = ver.get("protocol").and_then(|x| x.as_f64()).unwrap_or(0.0) as i64;
    }
    if let Some(f) = v.get("favicon").and_then(|x| x.as_str()) {
        if let Some(b) = f.split_once(',').map(|x| x.1) {
            st.icon = png::decode(&base64_decode(b)).ok();
        }
    }
    let t0 = Instant::now();
    send(&mut s, 1, &0x1234_5678_9abc_def0u64.to_be_bytes(), false)?;
    let (id, _) = recv(&mut s, false)?;
    if id == 1 {
        st.ping_ms = t0.elapsed().as_millis() as u32;
    }
    Ok(())
}

/// Kết nối đã đăng nhập xong, sẵn sàng vào giai đoạn cấu hình/chơi.
pub struct Conn {
    pub s: TcpStream,
    pub comp: bool,
}

pub enum Login {
    /// Đăng nhập xong (server chế độ offline hoặc không cần xác thực)
    Success { name: String, conn: Conn },
    /// Server từ chối, kèm lý do (đã đổi sang chữ có mã màu)
    Disconnected(String),
    /// Server bật xác thực Mojang (online-mode): cần mã hoá + token tài khoản
    NeedsAuth,
    Failed(String),
}

/// UUID chế độ offline: MD5("OfflinePlayer:" + tên) với bit phiên bản 3.
pub fn offline_uuid(name: &str) -> [u8; 16] {
    let mut h = md5(format!("OfflinePlayer:{name}").as_bytes());
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    h
}

pub fn login(addr: &str, name: &str) -> Login {
    match login_inner(addr, name) {
        Ok(l) => l,
        Err(e) => Login::Failed(e),
    }
}

fn login_inner(addr: &str, name: &str) -> Result<Login, String> {
    let mut s = connect(addr, 10)?;
    s.set_read_timeout(Some(Duration::from_secs(15))).ok();
    handshake(&mut s, addr, 2)?;
    let mut b = Vec::new();
    put_str(&mut b, name);
    b.extend_from_slice(&offline_uuid(name));
    send(&mut s, 0, &b, false)?;
    let mut comp = false;
    loop {
        let (id, body) = recv(&mut s, comp)?;
        let mut i = 0;
        match id {
            // Disconnect (login): JSON text
            0 => {
                let t = rd_str(&body, &mut i).unwrap_or_default();
                let v = json::parse(&t).unwrap_or(Value::Str(t));
                return Ok(Login::Disconnected(component_to_legacy(&v)));
            }
            // Encryption Request
            1 => return Ok(Login::NeedsAuth),
            // Login Success
            2 => {
                // UUID(16) + tên + thuộc tính
                let name2 = if body.len() > 16 {
                    let mut j = 16;
                    rd_str(&body, &mut j).unwrap_or_else(|| name.to_string())
                } else {
                    name.to_string()
                };
                // Login Acknowledged
                send(&mut s, 3, &[], comp)?;
                s.set_read_timeout(None).ok();
                return Ok(Login::Success { name: name2, conn: Conn { s, comp } });
            }
            // Set Compression
            3 => {
                let th = rd_varint(&body, &mut i).unwrap_or(-1);
                comp = th >= 0;
            }
            // Login Plugin Request: trả lời "không hiểu"
            4 => {
                let mid = rd_varint(&body, &mut i).unwrap_or(0);
                let mut r = Vec::new();
                put_varint(&mut r, mid as u32);
                r.push(0);
                send(&mut s, 2, &r, comp)?;
            }
            _ => {}
        }
    }
}

/// MD5 (RFC 1321) cho UUID offline.
pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&((data.len() as u64) * 8).to_le_bytes());
    for ch in m.chunks(64) {
        let w: Vec<u32> = (0..16).map(|i| u32::from_le_bytes([ch[i * 4], ch[i * 4 + 1], ch[i * 4 + 2], ch[i * 4 + 3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (mut f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(w[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..].copy_from_slice(&d0.to_le_bytes());
    out
}
