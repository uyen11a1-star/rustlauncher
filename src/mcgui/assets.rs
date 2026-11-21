//! Nạp tài nguyên GUI (ảnh, sprite 9 mảnh, font, ngôn ngữ, câu splash) từ một nguồn bất kỳ (client.jar hoặc thư mục).
use std::collections::HashMap;

use super::font::Font;
use crate::{json, png};

pub trait Source: Sync {
    fn read(&self, path: &str) -> Option<Vec<u8>>;
}

#[derive(Clone, Copy, Debug)]
pub enum Slice {
    Stretch,
    /// cắt 9 mảnh: biên trái, trên, phải, dưới, kích thước gốc của sprite; phần giữa được lát lại như Minecraft
    Nine { l: f32, t: f32, r: f32, b: f32 },
    Tile,
}

#[derive(Clone, Copy)]
pub struct Sprite {
    pub tex: usize,
    pub w: f32,
    pub h: f32,
    pub slice: Slice,
}

pub struct GuiAssets {
    pub imgs: Vec<png::Image>,
    pub sprites: HashMap<String, Sprite>,
    pub font: Font,
    pub lang: HashMap<String, String>,
    pub splashes: Vec<String>,
    pub pano: Option<[usize; 6]>,
    /// nguồn panorama đang dùng (để ghi nhật ký)
    pub pano_note: &'static str,
    pub logo: Option<usize>,
    pub edition: Option<usize>,
    pub mojang: Option<usize>,
    pub bg: Option<usize>,
    pub list_bg: Option<usize>,
    pub header_sep: Option<usize>,
    pub footer_sep: Option<usize>,
    pub world_icon: Option<usize>,
    pub version: String,
}

/// Sprite cần dùng (đường dẫn trong textures/gui/sprites/, không đuôi).
const SPRITES: &[&str] = &[
    "widget/button",
    "widget/button_highlighted",
    "widget/button_disabled",
    "widget/slider",
    "widget/slider_highlighted",
    "widget/slider_handle",
    "widget/slider_handle_highlighted",
    "widget/text_field",
    "widget/text_field_highlighted",
    "widget/checkbox",
    "widget/checkbox_highlighted",
    "widget/checkbox_selected",
    "widget/checkbox_selected_highlighted",
    "widget/scroller",
    "widget/scroller_background",
    "widget/tab",
    "widget/tab_highlighted",
    "widget/tab_selected",
    "widget/tab_selected_highlighted",
    "widget/cross_button",
    "widget/cross_button_highlighted",
    "widget/locked_button",
    "widget/unlocked_button",
    "world_list/join",
    "world_list/join_highlighted",
    "world_list/marked_join",
    "world_list/marked_join_highlighted",
    "world_list/warning",
    "world_list/warning_highlighted",
    "world_list/error",
    "world_list/error_highlighted",
    "icon/language",
    "icon/accessibility",
    "icon/search",
    "icon/checkmark",
    "icon/info",
    "icon/ping_5",
    "icon/ping_unknown",
    "widget/text_field_highlighted",
    "server_list/incompatible",
    "server_list/unreachable",
    "server_list/ping_1",
    "server_list/ping_2",
    "server_list/ping_3",
    "server_list/ping_4",
    "server_list/ping_5",
    "server_list/pinging_1",
    "server_list/pinging_2",
    "server_list/pinging_3",
    "server_list/pinging_4",
    "server_list/pinging_5",
    "server_list/join",
    "server_list/join_highlighted",
    "server_list/move_up",
    "server_list/move_down",
    "tooltip/background",
    "tooltip/frame",
];

fn parse_slice(src: &dyn Source, path: &str, w: f32, h: f32) -> Slice {
    let Some(b) = src.read(&format!("{path}.mcmeta")) else { return Slice::Stretch };
    let Ok(text) = String::from_utf8(b) else { return Slice::Stretch };
    let Ok(v) = json::parse(&text) else { return Slice::Stretch };
    let Some(sc) = v.get("gui").and_then(|g| g.get("scaling")) else { return Slice::Stretch };
    match sc.get("type").and_then(|t| t.as_str()) {
        Some("nine_slice") => {
            let _ = (w, h);
            let num = |v: Option<&json::Value>| v.and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
            match sc.get("border") {
                Some(json::Value::Num(n)) => {
                    let n = *n as f32;
                    Slice::Nine { l: n, t: n, r: n, b: n }
                }
                Some(o) => Slice::Nine { l: num(o.get("left")), t: num(o.get("top")), r: num(o.get("right")), b: num(o.get("bottom")) },
                None => Slice::Stretch,
            }
        }
        Some("tile") => Slice::Tile,
        _ => Slice::Stretch,
    }
}

impl GuiAssets {
    pub fn img(&self, t: usize) -> &png::Image {
        &self.imgs[t]
    }

    pub fn tr(&self, key: &str) -> String {
        self.lang.get(key).cloned().unwrap_or_else(|| key.to_string())
    }

    /// Nạp mọi thứ. `progress(done, total)` được gọi sau mỗi tài nguyên để vẽ thanh tiến trình thật.
    pub fn blank(version: &str) -> GuiAssets {
        GuiAssets {
            imgs: Vec::new(),
            sprites: HashMap::new(),
            font: Font::default(),
            lang: HashMap::new(),
            splashes: Vec::new(),
            pano: None,
            pano_note: "",
            logo: None,
            edition: None,
            mojang: None,
            bg: None,
            list_bg: None,
            header_sep: None,
            footer_sep: None,
            world_icon: None,
            version: version.to_string(),
        }
    }

    /// Chỉ logo Mojang (để vẽ màn đỏ ngay khi chưa nạp xong phần còn lại).
    pub fn mini(src: &dyn Source, version: &str) -> GuiAssets {
        let mut a = GuiAssets::blank(version);
        if let Some(img) = src.read("assets/minecraft/textures/gui/title/mojangstudios.png").and_then(|b| png::decode(&b).ok()) {
            a.imgs.push(img);
            a.mojang = Some(0);
        }
        a
    }

    pub fn load(src: &dyn Source, version: &str, progress: &dyn Fn(usize, usize)) -> GuiAssets {
        let mut a = GuiAssets::blank(version);
        let total = SPRITES.len() + 6 + 8;
        let mut done = 0usize;
        let mut tick = |done: &mut usize| {
            *done += 1;
            progress(*done, total);
        };
        let mut push = |a: &mut GuiAssets, path: &str| -> Option<usize> {
            let b = src.read(path)?;
            let img = png::decode(&b).ok()?;
            a.imgs.push(img);
            Some(a.imgs.len() - 1)
        };
        let base = "assets/minecraft/textures/gui/";
        a.mojang = push(&mut a, &format!("{base}title/mojangstudios.png"));
        tick(&mut done);
        a.font = Font::load(src, &mut a.imgs);
        tick(&mut done);
        for name in SPRITES {
            let path = format!("{base}sprites/{name}.png");
            if let Some(t) = push(&mut a, &path) {
                let (w, h) = (a.imgs[t].w as f32, a.imgs[t].h as f32);
                let slice = parse_slice(src, &path, w, h);
                a.sprites.insert(name.to_string(), Sprite { tex: t, w, h, slice });
            }
            tick(&mut done);
        }
        a.logo = push(&mut a, &format!("{base}title/minecraft.png"));
        a.edition = push(&mut a, &format!("{base}title/edition.png"));
        a.bg = push(&mut a, &format!("{base}menu_background.png"));
        a.list_bg = push(&mut a, &format!("{base}menu_list_background.png"));
        a.header_sep = push(&mut a, &format!("{base}header_separator.png"));
        a.footer_sep = push(&mut a, &format!("{base}footer_separator.png"));
        a.world_icon = push(&mut a, "assets/minecraft/textures/misc/unknown_server.png");
        tick(&mut done);
        if let Some(b) = src.read("assets/minecraft/lang/en_us.json") {
            if let Some(v) = String::from_utf8(b).ok().and_then(|s| json::parse(&s).ok()) {
                if let json::Value::Obj(m) = v {
                    for (k, v) in m {
                        if let Some(s) = v.as_str() {
                            a.lang.insert(k, s.to_string());
                        }
                    }
                }
            }
        }
        if let Some(b) = src.read("assets/minecraft/texts/splashes.txt") {
            a.splashes = String::from_utf8_lossy(&b).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        }
        tick(&mut done);
        // panorama: ưu tiên ảnh thật (asset index/jar), ảnh giữ chỗ 1x1 bị loại, cuối cùng dựng bầu trời tạm
        let (faces, note) = crate::mcgui::pano::load(src);
        let mut ids = [0usize; 6];
        for (i, f) in faces.into_iter().enumerate().take(6) {
            a.imgs.push(f);
            ids[i] = a.imgs.len() - 1;
        }
        a.pano = Some(ids);
        a.pano_note = note;
        for _ in 0..6 {
            tick(&mut done);
        }
        progress(total, total);
        a
    }
}
