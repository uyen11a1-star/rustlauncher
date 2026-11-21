//! Nguồn ảnh panorama màn hình chính.
//! Thứ tự ưu tiên: ảnh thật tải từ asset index (thư mục riêng) > ảnh trong client.jar > bầu trời dựng tạm.
//! Ảnh nhỏ hơn `MIN_FACE` (vd file giữ chỗ 1x1 trong jar) bị coi là thiếu, không bao giờ được vẽ ra màu xám.
use std::path::PathBuf;

use serde::Deserialize;

use crate::{mcgui::assets::Source, png::Image};

pub const MIN_FACE: usize = 64;
pub const KEY: &str = "assets/minecraft/textures/gui/title/background/panorama_";

/// Đọc `panorama_N.png` từ `dir` trước, các đường dẫn khác (hoặc khi thiếu file) đọc từ `jar`.
pub struct Layered<'a> {
    pub jar: &'a dyn Source,
    pub dir: PathBuf,
}

impl Source for Layered<'_> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        if let Some(rest) = path.strip_prefix(KEY) {
            if let Ok(b) = std::fs::read(self.dir.join(format!("panorama_{rest}"))) {
                return Some(b);
            }
        }
        self.jar.read(path)
    }
}

/// 6 mặt (thứ tự như shader: +z, +x, -z, -x, trên, dưới) và ghi chú nguồn để đưa vào nhật ký.
pub fn load(src: &dyn Source) -> (Vec<Image>, &'static str) {
    let paths: Vec<String> = (0..6).map(|i| format!("{KEY}{i}.png")).collect();
    let decoded: Vec<Option<Image>> = std::thread::scope(|sc| {
        let hs: Vec<_> = paths
            .iter()
            .map(|p| {
                sc.spawn(move || {
                    let b = src.read(p)?;
                    crate::png::decode(&b).ok()
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let ok = decoded.iter().all(|d| d.as_ref().map_or(false, |i| i.w >= MIN_FACE && i.h >= MIN_FACE));
    if ok {
        return (decoded.into_iter().flatten().collect(), "ảnh thật");
    }
    (fallback(), "bầu trời dựng tạm (không có ảnh panorama thật)")
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn face(n: usize, f: impl Fn(usize, usize) -> [f32; 3]) -> Image {
    let mut rgba = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let c = f(x, y);
            rgba.extend_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, 255]);
        }
    }
    Image { w: n, h: n, rgba }
}

/// Bầu trời xanh, chân trời sáng, mặt đất xanh lá: chỉ dùng khi hoàn toàn không có ảnh thật.
fn fallback() -> Vec<Image> {
    const N: usize = 128;
    let (sky, hor, ground) = ([96.0, 150.0, 230.0], [190.0, 215.0, 245.0], [84.0, 120.0, 64.0]);
    let side = |_x: usize, y: usize| {
        let t = y as f32 / (N - 1) as f32;
        if t < 0.5 {
            lerp(sky, hor, t * 2.0)
        } else {
            lerp(hor, ground, ((t - 0.5) * 2.0).min(1.0).powf(0.35))
        }
    };
    let mut v: Vec<Image> = (0..4).map(|_| face(N, side)).collect();
    v.push(face(N, |x, y| {
        let (dx, dy) = (x as f32 / (N - 1) as f32 - 0.5, y as f32 / (N - 1) as f32 - 0.5);
        lerp(sky, hor, ((dx * dx + dy * dy).sqrt() * 1.4).min(1.0))
    }));
    v.push(face(N, |_, _| ground));
    v
}

/// Lấy (số mặt, hash, kích thước) của 6 ảnh panorama từ JSON asset index của Mojang.
pub fn parse_objects(index_json: &[u8]) -> Vec<(usize, String, u64)> {
    #[derive(Deserialize)]
    struct Obj {
        hash: String,
        size: u64,
    }
    #[derive(Deserialize)]
    struct Idx {
        objects: std::collections::HashMap<String, Obj>,
    }
    let Ok(idx) = serde_json::from_slice::<Idx>(index_json) else {
        return Vec::new();
    };
    (0..6)
        .filter_map(|i| {
            let key = format!("minecraft/textures/gui/title/background/panorama_{i}.png");
            idx.objects.get(&key).filter(|o| o.hash.len() >= 2).map(|o| (i, o.hash.clone(), o.size))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// PNG RGB 8-bit không nén (khối deflate "stored") để test không cần file ngoài.
    fn make_png(w: usize, h: usize, rgb: [u8; 3]) -> Vec<u8> {
        let mut raw = Vec::new();
        for _ in 0..h {
            raw.push(0);
            for _ in 0..w {
                raw.extend_from_slice(&rgb);
            }
        }
        let mut z = vec![0x78, 0x01];
        let mut chunks = raw.chunks(65535).peekable();
        while let Some(c) = chunks.next() {
            z.push(if chunks.peek().is_none() { 1 } else { 0 });
            z.extend_from_slice(&(c.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
            z.extend_from_slice(c);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let mut chunk = |name: &[u8; 4], data: &[u8]| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut body = name.to_vec();
            body.extend_from_slice(data);
            out.extend_from_slice(&body);
            out.extend_from_slice(&crate::inflate::crc32(&body).to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(w as u32).to_be_bytes());
        ihdr.extend_from_slice(&(h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        chunk(b"IDAT", &z);
        chunk(b"IEND", &[]);
        out
    }

    struct Fake(HashMap<String, Vec<u8>>);
    impl Source for Fake {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.get(path).cloned()
        }
    }

    fn jar_with(w: usize, h: usize, rgb: [u8; 3]) -> Fake {
        Fake((0..6).map(|i| (format!("{KEY}{i}.png"), make_png(w, h, rgb))).collect())
    }

    #[test]
    fn real_faces_from_jar_are_used() {
        let (faces, note) = load(&jar_with(128, 128, [10, 20, 30]));
        assert_eq!(note, "ảnh thật");
        assert_eq!(faces.len(), 6);
        assert!(faces.iter().all(|f| f.w == 128 && f.rgba[0] == 10 && f.rgba[2] == 30));
    }

    #[test]
    fn placeholder_1x1_is_rejected_and_never_gray() {
        // đúng triệu chứng trong nhật ký: 6 ảnh 1x1 màu 98,111,113
        let (faces, note) = load(&jar_with(1, 1, [98, 111, 113]));
        assert!(note.starts_with("bầu trời dựng tạm"));
        assert_eq!(faces.len(), 6);
        assert!(faces.iter().all(|f| f.w >= MIN_FACE && f.h >= MIN_FACE));
        // mặt trên là trời xanh (xanh dương trội), không phải xám
        let top = &faces[4].rgba;
        assert!(top[2] > top[0] + 40, "mặt trên phải xanh trời, thấy {:?}", &top[..3]);
    }

    #[test]
    fn missing_face_falls_back() {
        let mut j = jar_with(128, 128, [1, 2, 3]);
        j.0.remove(&format!("{KEY}3.png"));
        assert!(load(&j).1.starts_with("bầu trời"));
    }

    #[test]
    fn layered_prefers_downloaded_faces_over_jar_placeholder() {
        let dir = std::env::temp_dir().join(format!("pano_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..6 {
            std::fs::write(dir.join(format!("panorama_{i}.png")), make_png(256, 256, [200, 100, 50])).unwrap();
        }
        let jar = jar_with(1, 1, [98, 111, 113]);
        let src = Layered { jar: &jar, dir: dir.clone() };
        let (faces, note) = load(&src);
        assert_eq!(note, "ảnh thật");
        assert!(faces.iter().all(|f| f.w == 256 && f.rgba[0] == 200));
        // đường dẫn khác vẫn đọc từ jar
        assert!(src.read("khong/co/file").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn layered_falls_back_to_jar_when_dir_empty() {
        let dir = std::env::temp_dir().join("pano_test_khong_ton_tai_xyz");
        let jar = jar_with(100, 100, [7, 8, 9]);
        let (faces, note) = load(&Layered { jar: &jar, dir });
        assert_eq!(note, "ảnh thật");
        assert_eq!(faces[0].w, 100);
    }

    #[test]
    fn parse_asset_index() {
        let j = br#"{"objects":{
            "minecraft/textures/gui/title/background/panorama_0.png":{"hash":"aabbccdd","size":123},
            "minecraft/textures/gui/title/background/panorama_5.png":{"hash":"11223344","size":456},
            "minecraft/textures/gui/title/background/panorama_2.png":{"hash":"x","size":9},
            "icons/icon_16x16.png":{"hash":"deadbeef","size":1}}}"#;
        let v = parse_objects(j);
        assert_eq!(v, vec![(0, "aabbccdd".to_string(), 123), (5, "11223344".to_string(), 456)]);
        assert!(parse_objects(b"khong phai json").is_empty());
    }
}
