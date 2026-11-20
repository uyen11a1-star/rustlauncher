//! Giải mã PNG tự viết (không xen kẽ): màu xám/RGB/bảng màu/kèm alpha, độ sâu 1-16 bit. Kết quả luôn là RGBA8.
use crate::inflate;

#[derive(Clone)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

fn be32(b: &[u8], o: usize) -> usize {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as usize
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i32, b as i32, c as i32);
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

pub fn decode(data: &[u8]) -> Result<Image, String> {
    if data.len() < 33 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("không phải PNG".into());
    }
    let (mut w, mut h, mut depth, mut ctype, mut interlace) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut plte: Vec<u8> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    let mut p = 8;
    while p + 12 <= data.len() {
        let len = be32(data, p);
        let kind = &data[p + 4..p + 8];
        let end = p + 8 + len;
        if end + 4 > data.len() {
            return Err("chunk PNG cụt".into());
        }
        let body = &data[p + 8..end];
        match kind {
            b"IHDR" => {
                if body.len() < 13 {
                    return Err("IHDR cụt".into());
                }
                w = be32(body, 0);
                h = be32(body, 4);
                depth = body[8];
                ctype = body[9];
                interlace = body[12];
            }
            b"PLTE" => plte = body.to_vec(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        p = end + 4;
    }
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err("kích thước PNG sai".into());
    }
    if interlace != 0 {
        return Err("PNG xen kẽ chưa hỗ trợ".into());
    }
    let channels = match ctype {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err("kiểu màu PNG sai".into()),
    };
    if !matches!(depth, 1 | 2 | 4 | 8 | 16) {
        return Err("độ sâu bit PNG sai".into());
    }
    let bpp_bits = channels * depth as usize;
    let row_bytes = (w * bpp_bits + 7) / 8;
    let bpp = (bpp_bits + 7) / 8;
    if idat.len() < 6 {
        return Err("thiếu IDAT".into());
    }
    // zlib: 2 byte header, deflate, 4 byte adler (bỏ qua adler)
    let raw = inflate::inflate(&idat[2..], h * (row_bytes + 1))?;

    // bỏ lọc từng dòng
    let mut img = vec![0u8; h * row_bytes];
    for y in 0..h {
        let f = raw[y * (row_bytes + 1)];
        let src = &raw[y * (row_bytes + 1) + 1..(y + 1) * (row_bytes + 1)];
        let (before, rest) = img.split_at_mut(y * row_bytes);
        let cur = &mut rest[..row_bytes];
        let prev: Option<&[u8]> = if y > 0 { Some(&before[(y - 1) * row_bytes..]) } else { None };
        for i in 0..row_bytes {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prev.map_or(0, |pr| pr[i]);
            let c = if i >= bpp { prev.map_or(0, |pr| pr[i - bpp]) } else { 0 };
            cur[i] = match f {
                0 => src[i],
                1 => src[i].wrapping_add(a),
                2 => src[i].wrapping_add(b),
                3 => src[i].wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => src[i].wrapping_add(paeth(a, b, c)),
                _ => return Err("filter PNG sai".into()),
            };
        }
    }

    // sang RGBA8
    let mut rgba = vec![255u8; w * h * 4];
    let sample = |row: &[u8], idx: usize| -> u32 {
        // lấy mẫu thứ idx (tính theo đơn vị sample) của dòng
        match depth {
            8 => row[idx] as u32,
            16 => ((row[idx * 2] as u32) << 8) | row[idx * 2 + 1] as u32,
            d => {
                let bit = idx * d as usize;
                let byte = row[bit / 8];
                let shift = 8 - d as usize - (bit % 8);
                ((byte >> shift) & ((1u8 << d) - 1)) as u32
            }
        }
    };
    let max = if depth == 16 { 65535u32 } else { (1u32 << depth) - 1 };
    let to8 = |v: u32| -> u8 { ((v * 255 + max / 2) / max) as u8 };
    for y in 0..h {
        let row = &img[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..w {
            let o = (y * w + x) * 4;
            match ctype {
                0 => {
                    let raw_g = sample(row, x);
                    let g = to8(raw_g);
                    rgba[o..o + 3].copy_from_slice(&[g, g, g]);
                    // tRNS của ảnh xám: một giá trị xám duy nhất là trong suốt
                    if trns.len() >= 2 && raw_g == (((trns[0] as u32) << 8) | trns[1] as u32) {
                        rgba[o + 3] = 0;
                    }
                }
                2 => {
                    let mut hit = trns.len() >= 6;
                    for k in 0..3 {
                        let v = sample(row, x * 3 + k);
                        rgba[o + k] = to8(v);
                        if hit && v != (((trns[k * 2] as u32) << 8) | trns[k * 2 + 1] as u32) {
                            hit = false;
                        }
                    }
                    if hit {
                        rgba[o + 3] = 0;
                    }
                }
                3 => {
                    let i = sample(row, x) as usize;
                    if i * 3 + 2 >= plte.len() {
                        return Err("chỉ số bảng màu vượt PLTE".into());
                    }
                    rgba[o..o + 3].copy_from_slice(&plte[i * 3..i * 3 + 3]);
                    rgba[o + 3] = trns.get(i).copied().unwrap_or(255);
                }
                4 => {
                    let g = to8(sample(row, x * 2));
                    rgba[o..o + 3].copy_from_slice(&[g, g, g]);
                    rgba[o + 3] = to8(sample(row, x * 2 + 1));
                }
                _ => {
                    for k in 0..4 {
                        rgba[o + k] = to8(sample(row, x * 4 + k));
                    }
                }
            }
        }
    }
    Ok(Image { w, h, rgba })
}
