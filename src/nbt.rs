//! Đọc NBT (định dạng dữ liệu của Minecraft Java, big-endian) tự viết.
use std::collections::HashMap;

pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Bytes(Vec<u8>),
    Str(String),
    List(Vec<Tag>),
    Compound(HashMap<String, Tag>),
    Ints(Vec<i32>),
    Longs(Vec<i64>),
}

impl Tag {
    pub fn get(&self, k: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(m) => m.get(k),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        Some(match self {
            Tag::Byte(v) => *v as i64,
            Tag::Short(v) => *v as i64,
            Tag::Int(v) => *v as i64,
            Tag::Long(v) => *v,
            _ => return None,
        })
    }
    pub fn as_f64(&self) -> Option<f64> {
        Some(match self {
            Tag::Float(v) => *v as f64,
            Tag::Double(v) => *v,
            other => other.as_i64()? as f64,
        })
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Option<&[Tag]> {
        match self {
            Tag::List(l) => Some(l),
            _ => None,
        }
    }
    pub fn as_longs(&self) -> Option<&[i64]> {
        match self {
            Tag::Longs(l) => Some(l),
            _ => None,
        }
    }
    pub fn as_ints(&self) -> Option<&[i32]> {
        match self {
            Tag::Ints(l) => Some(l),
            _ => None,
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.p + n > self.b.len() {
            return Err("NBT cụt".into());
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Result<i16, String> {
        let b = self.take(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Result<i32, String> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i64(&mut self) -> Result<i64, String> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(i64::from_be_bytes(a))
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.i16()? as u16 as usize;
        let b = self.take(n)?;
        // NBT dùng "modified UTF-8"; với tên khối/khóa thường chỉ là ASCII/UTF-8 thường.
        Ok(String::from_utf8_lossy(b).into_owned())
    }
    fn len(&mut self, elem: usize) -> Result<usize, String> {
        let n = self.i32()?;
        if n < 0 || (n as usize).saturating_mul(elem) > self.b.len() - self.p {
            return Err("độ dài NBT sai".into());
        }
        Ok(n as usize)
    }

    fn payload(&mut self, ty: u8, depth: u32) -> Result<Tag, String> {
        if depth > 64 {
            return Err("NBT lồng quá sâu".into());
        }
        Ok(match ty {
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(self.i16()?),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(f32::from_bits(self.i32()? as u32)),
            6 => Tag::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let n = self.len(1)?;
                Tag::Bytes(self.take(n)?.to_vec())
            }
            8 => Tag::Str(self.string()?),
            9 => {
                let et = self.u8()?;
                let n = self.len(if et == 0 { 0 } else { 1 })?;
                let mut v = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    v.push(self.payload(et, depth + 1)?);
                }
                Tag::List(v)
            }
            10 => {
                let mut m = HashMap::new();
                loop {
                    let t = self.u8()?;
                    if t == 0 {
                        break;
                    }
                    let name = self.string()?;
                    m.insert(name, self.payload(t, depth + 1)?);
                }
                Tag::Compound(m)
            }
            11 => {
                let n = self.len(4)?;
                let raw = self.take(n * 4)?;
                Tag::Ints(raw.chunks_exact(4).map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect())
            }
            12 => {
                let n = self.len(8)?;
                let raw = self.take(n * 8)?;
                Tag::Longs(
                    raw.chunks_exact(8)
                        .map(|c| {
                            let mut a = [0u8; 8];
                            a.copy_from_slice(c);
                            i64::from_be_bytes(a)
                        })
                        .collect(),
                )
            }
            _ => return Err(format!("kiểu NBT lạ: {ty}")),
        })
    }
}

/// Đọc tag gốc (compound có tên) của một khối NBT đã giải nén.
pub fn parse(data: &[u8]) -> Result<Tag, String> {
    let mut r = R { b: data, p: 0 };
    let ty = r.u8()?;
    if ty != 10 {
        return Err("NBT gốc không phải compound".into());
    }
    let _name = r.string()?;
    r.payload(10, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_compound() {
        // compound "" { byte a=5, string s="hi", long-array l=[1,2] }
        let mut d = vec![10, 0, 0];
        d.extend([1, 0, 1, b'a', 5]);
        d.extend([8, 0, 1, b's', 0, 2, b'h', b'i']);
        d.extend([12, 0, 1, b'l', 0, 0, 0, 2]);
        d.extend(1i64.to_be_bytes());
        d.extend(2i64.to_be_bytes());
        d.push(0);
        let t = parse(&d).unwrap();
        assert_eq!(t.get("a").unwrap().as_i64(), Some(5));
        assert_eq!(t.get("s").unwrap().as_str(), Some("hi"));
        assert_eq!(t.get("l").unwrap().as_longs().unwrap(), &[1, 2]);
    }
}
