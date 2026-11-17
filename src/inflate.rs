//! Giải nén deflate (RFC 1951) tự viết, theo cấu trúc Huffman chuẩn (như puff.c) + CRC32.

const LBASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DEXT: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

struct Huff {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huff {
    fn new(lengths: &[u8]) -> Result<Huff, String> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        // kiểm tra tập mã không bị quá đầy
        let mut left: i32 = 1;
        for len in 1..16 {
            left <<= 1;
            left -= count[len] as i32;
            if left < 0 {
                return Err("bảng Huffman quá đầy".into());
            }
        }
        let mut offs = [0u16; 16];
        for len in 1..15 {
            offs[len + 1] = offs[len] + count[len];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Huff { count, symbol })
    }
}

struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    buf: u32,
    n: u32,
}

impl<'a> Bits<'a> {
    fn get(&mut self, need: u32) -> Result<u32, String> {
        while self.n < need {
            let b = *self.src.get(self.pos).ok_or("deflate: hết dữ liệu")?;
            self.pos += 1;
            self.buf |= (b as u32) << self.n;
            self.n += 8;
        }
        let v = if need == 0 { 0 } else { self.buf & ((1u32 << need) - 1) };
        self.buf >>= need;
        self.n -= need;
        Ok(v)
    }

    fn decode(&mut self, h: &Huff) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= self.get(1)? as i32;
            let count = h.count[len] as i32;
            if code - count < first {
                return Ok(h.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("deflate: mã Huffman sai".into())
    }
}

fn codes(b: &mut Bits, out: &mut Vec<u8>, lit: &Huff, dist: &Huff, limit: usize) -> Result<(), String> {
    loop {
        let sym = b.decode(lit)? as usize;
        if sym < 256 {
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let s = sym - 257;
            if s >= 29 {
                return Err("deflate: mã độ dài sai".into());
            }
            let len = LBASE[s] as usize + b.get(LEXT[s] as u32)? as usize;
            let ds = b.decode(dist)? as usize;
            if ds >= 30 {
                return Err("deflate: mã khoảng cách sai".into());
            }
            let d = DBASE[ds] as usize + b.get(DEXT[ds] as u32)? as usize;
            if d > out.len() {
                return Err("deflate: khoảng cách vượt đầu ra".into());
            }
            let start = out.len() - d;
            for i in 0..len {
                let c = out[start + i];
                out.push(c);
            }
        }
        if out.len() > limit {
            return Err("deflate: đầu ra lớn hơn khai báo".into());
        }
    }
}

/// Giải nén luồng deflate thô. `expected` là kích thước sau giải nén (dùng làm giới hạn và kiểm tra).
pub fn inflate(src: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    inflate_impl(src, expected, true)
}

/// Giải nén khi không biết trước kích thước; `max` là giới hạn an toàn.
pub fn inflate_any(src: &[u8], max: usize) -> Result<Vec<u8>, String> {
    inflate_impl(src, max, false)
}

/// Bỏ header zlib (2 byte) rồi giải nén.
pub fn zlib_any(src: &[u8], max: usize) -> Result<Vec<u8>, String> {
    if src.len() < 3 {
        return Err("zlib cụt".into());
    }
    inflate_any(&src[2..], max)
}

/// Bỏ header gzip rồi giải nén.
pub fn gzip_any(src: &[u8], max: usize) -> Result<Vec<u8>, String> {
    if src.len() < 18 || src[0] != 0x1f || src[1] != 0x8b || src[2] != 8 {
        return Err("không phải gzip".into());
    }
    let flags = src[3];
    let mut p = 10usize;
    if flags & 4 != 0 {
        if p + 2 > src.len() {
            return Err("gzip cụt".into());
        }
        p += 2 + u16::from_le_bytes([src[p], src[p + 1]]) as usize;
    }
    for bit in [8u8, 16u8] {
        if flags & bit != 0 {
            while p < src.len() && src[p] != 0 {
                p += 1;
            }
            p += 1;
        }
    }
    if flags & 2 != 0 {
        p += 2;
    }
    if p >= src.len() {
        return Err("gzip cụt".into());
    }
    inflate_any(&src[p..], max)
}

fn inflate_impl(src: &[u8], expected: usize, exact: bool) -> Result<Vec<u8>, String> {
    let mut b = Bits { src, pos: 0, buf: 0, n: 0 };
    let mut out: Vec<u8> = Vec::with_capacity(if exact { expected } else { 1 << 16 });
    loop {
        let last = b.get(1)?;
        match b.get(2)? {
            0 => {
                b.buf = 0;
                b.n = 0;
                if b.pos + 4 > src.len() {
                    return Err("deflate: khối stored cụt".into());
                }
                let len = u16::from_le_bytes([src[b.pos], src[b.pos + 1]]) as usize;
                let nlen = u16::from_le_bytes([src[b.pos + 2], src[b.pos + 3]]) as usize;
                if len != (!nlen & 0xffff) {
                    return Err("deflate: LEN/NLEN không khớp".into());
                }
                b.pos += 4;
                if b.pos + len > src.len() {
                    return Err("deflate: khối stored cụt".into());
                }
                out.extend_from_slice(&src[b.pos..b.pos + len]);
                b.pos += len;
            }
            1 => {
                let mut l = [0u8; 288];
                for (i, x) in l.iter_mut().enumerate() {
                    *x = if i < 144 { 8 } else if i < 256 { 9 } else if i < 280 { 7 } else { 8 };
                }
                let lit = Huff::new(&l)?;
                let dist = Huff::new(&[5u8; 30])?;
                codes(&mut b, &mut out, &lit, &dist, expected)?;
            }
            2 => {
                let nlen = b.get(5)? as usize + 257;
                let ndist = b.get(5)? as usize + 1;
                let ncode = b.get(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err("deflate: số mã sai".into());
                }
                let mut lengths = [0u8; 320];
                for &o in ORDER.iter().take(ncode) {
                    lengths[o] = b.get(3)? as u8;
                }
                let lencode = Huff::new(&lengths[..19])?;
                let mut idx = 0;
                let mut ll = [0u8; 320];
                while idx < nlen + ndist {
                    let sym = b.decode(&lencode)?;
                    if sym < 16 {
                        ll[idx] = sym as u8;
                        idx += 1;
                    } else {
                        let (rep, val) = match sym {
                            16 => {
                                if idx == 0 {
                                    return Err("deflate: lặp không có giá trị trước".into());
                                }
                                (3 + b.get(2)? as usize, ll[idx - 1])
                            }
                            17 => (3 + b.get(3)? as usize, 0),
                            _ => (11 + b.get(7)? as usize, 0),
                        };
                        if idx + rep > nlen + ndist {
                            return Err("deflate: lặp vượt số mã".into());
                        }
                        for _ in 0..rep {
                            ll[idx] = val;
                            idx += 1;
                        }
                    }
                }
                if ll[256] == 0 {
                    return Err("deflate: thiếu mã kết thúc khối".into());
                }
                let lit = Huff::new(&ll[..nlen])?;
                let dist = Huff::new(&ll[nlen..nlen + ndist])?;
                codes(&mut b, &mut out, &lit, &dist, expected)?;
            }
            _ => return Err("deflate: loại khối sai".into()),
        }
        if last == 1 {
            break;
        }
    }
    if exact && out.len() != expected {
        return Err(format!("deflate: ra {} byte, cần {}", out.len(), expected));
    }
    Ok(out)
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
        }
        table[i as usize] = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}
