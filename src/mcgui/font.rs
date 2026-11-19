//! Font bitmap của Minecraft: đọc font/default.json (kèm các file include), cắt ô ký tự, đo bề rộng thật của từng ký tự.
use std::collections::HashMap;

use super::assets::Source;
use crate::{json, json::Value, png};

#[derive(Clone, Copy)]
pub struct Glyph {
    pub tex: usize,
    /// vùng trong ảnh (điểm ảnh)
    pub u: f32,
    pub v: f32,
    pub w: f32,
    pub h: f32,
    /// bước tiến theo đơn vị GUI
    pub adv: f32,
    /// độ dịch xuống so với đỉnh dòng chữ
    pub yoff: f32,
    /// kích thước vẽ (đơn vị GUI)
    pub dw: f32,
    pub dh: f32,
}

#[derive(Default)]
pub struct Font {
    pub glyphs: HashMap<char, Glyph>,
}

fn read_json(src: &dyn Source, path: &str) -> Option<Value> {
    let b = src.read(path)?;
    json::parse(&String::from_utf8(b).ok()?).ok()
}

impl Font {
    pub fn load(src: &dyn Source, imgs: &mut Vec<png::Image>) -> Font {
        let mut f = Font::default();
        f.provider_file(src, imgs, "assets/minecraft/font/default.json", 0);
        f
    }

    fn provider_file(&mut self, src: &dyn Source, imgs: &mut Vec<png::Image>, path: &str, depth: u32) {
        if depth > 4 {
            return;
        }
        let Some(v) = read_json(src, path) else { return };
        let Some(list) = v.get("providers").and_then(|p| p.as_arr()) else { return };
        for p in list {
            // bộ lọc uniform: chỉ nhận khi "uniform" là false (font mặc định)
            if let Some(flt) = p.get("filter") {
                if flt.get("uniform").and_then(|u| u.as_bool()) == Some(true) {
                    continue;
                }
            }
            match p.get("type").and_then(|t| t.as_str()) {
                Some("reference") => {
                    if let Some(id) = p.get("id").and_then(|i| i.as_str()) {
                        let name = id.trim_start_matches("minecraft:");
                        self.provider_file(src, imgs, &format!("assets/minecraft/font/{name}.json"), depth + 1);
                    }
                }
                Some("space") => {
                    if let Some(Value::Obj(m)) = p.get("advances") {
                        for (k, v) in m {
                            if let (Some(c), Some(a)) = (k.chars().next(), v.as_f64()) {
                                self.glyphs.entry(c).or_insert(Glyph { tex: 0, u: 0.0, v: 0.0, w: 0.0, h: 0.0, adv: a as f32, yoff: 0.0, dw: 0.0, dh: 0.0 });
                            }
                        }
                    }
                }
                Some("bitmap") => self.bitmap(src, imgs, p),
                _ => {}
            }
        }
    }

    fn bitmap(&mut self, src: &dyn Source, imgs: &mut Vec<png::Image>, p: &Value) {
        let (Some(file), Some(rows)) = (p.get("file").and_then(|f| f.as_str()), p.get("chars").and_then(|c| c.as_arr())) else { return };
        let height = p.get("height").and_then(|h| h.as_f64()).unwrap_or(8.0) as f32;
        let ascent = p.get("ascent").and_then(|h| h.as_f64()).unwrap_or(7.0) as f32;
        let name = file.trim_start_matches("minecraft:");
        let Some(b) = src.read(&format!("assets/minecraft/textures/{name}")) else { return };
        let Ok(img) = png::decode(&b) else { return };
        let grid: Vec<Vec<char>> = rows.iter().filter_map(|r| r.as_str()).map(|s| s.chars().collect()).collect();
        let ncols = grid.iter().map(|r| r.len()).max().unwrap_or(0);
        if ncols == 0 || grid.is_empty() {
            return;
        }
        let (cw, ch) = (img.w / ncols, img.h / grid.len());
        if cw == 0 || ch == 0 {
            return;
        }
        let scale = height / ch as f32;
        let tex = imgs.len();
        for (r, row) in grid.iter().enumerate() {
            for (c, &chr) in row.iter().enumerate() {
                if chr == '\0' || self.glyphs.contains_key(&chr) {
                    continue;
                }
                // bề rộng thật: cột phải nhất còn điểm ảnh không trong suốt
                let mut actual = 0usize;
                'cols: for x in (0..cw).rev() {
                    for y in 0..ch {
                        if img.rgba[((r * ch + y) * img.w + c * cw + x) * 4 + 3] != 0 {
                            actual = x + 1;
                            break 'cols;
                        }
                    }
                }
                if actual == 0 {
                    continue;
                }
                self.glyphs.insert(
                    chr,
                    Glyph {
                        tex,
                        u: (c * cw) as f32,
                        v: (r * ch) as f32,
                        w: actual as f32,
                        h: ch as f32,
                        adv: actual as f32 * scale + 1.0,
                        yoff: 7.0 - ascent,
                        dw: actual as f32 * scale,
                        dh: ch as f32 * scale,
                    },
                );
            }
        }
        imgs.push(img);
    }

    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c).or_else(|| self.glyphs.get(&'?'))
    }

    /// Bề rộng chuỗi (đơn vị GUI), bỏ qua mã định dạng §x.
    pub fn width(&self, s: &str) -> f32 {
        let mut w = 0.0;
        let mut it = s.chars();
        while let Some(c) = it.next() {
            if c == '§' {
                it.next();
                continue;
            }
            if let Some(g) = self.glyph(c) {
                w += g.adv;
            }
        }
        w
    }
}
