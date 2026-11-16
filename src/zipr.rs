//! Đọc zip/APK từng phần: parse EOCD + central directory từ byte đã tải bằng HTTP Range, rồi lấy từng entry.
use crate::inflate;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub crc: u32,
    pub comp_size: u64,
    pub size: u64,
    pub local_off: u64,
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Tìm EOCD trong phần đuôi file; trả về (offset central directory, kích thước).
pub fn find_cd(tail: &[u8]) -> Option<(u64, u64)> {
    if tail.len() < 22 {
        return None;
    }
    let mut i = tail.len() - 22;
    loop {
        if u32le(tail, i) == 0x0605_4b50 {
            let cd_size = u32le(tail, i + 12);
            let cd_off = u32le(tail, i + 16);
            if cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF {
                return None; // zip64: chưa hỗ trợ
            }
            return Some((cd_off as u64, cd_size as u64));
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

pub fn parse_central(cd: &[u8]) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 46 <= cd.len() {
        if u32le(cd, p) != 0x0201_4b50 {
            break;
        }
        let n = u16le(cd, p + 28) as usize;
        let m = u16le(cd, p + 30) as usize;
        let k = u16le(cd, p + 32) as usize;
        if p + 46 + n > cd.len() {
            return Err("central directory cụt".into());
        }
        out.push(Entry {
            name: String::from_utf8_lossy(&cd[p + 46..p + 46 + n]).into_owned(),
            method: u16le(cd, p + 10),
            crc: u32le(cd, p + 16),
            comp_size: u32le(cd, p + 20) as u64,
            size: u32le(cd, p + 24) as u64,
            local_off: u32le(cd, p + 42) as u64,
        });
        p += 46 + n + m + k;
    }
    if out.is_empty() {
        return Err("central directory rỗng".into());
    }
    Ok(out)
}

/// `buf` bắt đầu đúng tại local header của entry. Trả về nội dung đã giải nén và kiểm CRC.
pub fn entry_data(buf: &[u8], e: &Entry) -> Result<Vec<u8>, String> {
    if buf.len() < 30 || u32le(buf, 0) != 0x0403_4b50 {
        return Err(format!("local header sai cho {}", e.name));
    }
    let start = 30 + u16le(buf, 26) as usize + u16le(buf, 28) as usize;
    let end = start + e.comp_size as usize;
    if end > buf.len() {
        return Err(format!("dữ liệu {} bị cụt", e.name));
    }
    let raw = &buf[start..end];
    let data = match e.method {
        0 => raw.to_vec(),
        8 => inflate::inflate(raw, e.size as usize)?,
        m => return Err(format!("{}: phương thức nén {m} chưa hỗ trợ", e.name)),
    };
    if inflate::crc32(&data) != e.crc {
        return Err(format!("{}: CRC sai", e.name));
    }
    Ok(data)
}
