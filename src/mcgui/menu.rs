//! Các màn hình của Minecraft Java (immediate mode): màn Mojang, màn hình chính, chọn world, tùy chọn, tạm dừng...
//! Toạ độ và bố cục theo đúng công thức của vanilla (đơn vị GUI).
use super::canvas::*;

/// Một lần chạm đã xử lý ở lớp trên: toạ độ theo đơn vị GUI.
#[derive(Clone, Copy, Default)]
pub struct Ptr {
    /// ngón tay đang chạm (đang giữ)
    pub down: Option<(f32, f32)>,
    /// vừa nhấc ngón tay trong khung hình này mà không kéo: coi là bấm
    pub tap: Option<(f32, f32)>,
    /// vừa chạm xuống trong khung hình này
    pub press: Option<(f32, f32)>,
    /// dịch chuyển dọc của ngón tay từ khung trước (đơn vị GUI)
    pub dy: f32,
}

pub fn inside(r: [f32; 4], p: Option<(f32, f32)>) -> bool {
    p.map_or(false, |p| p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3])
}

pub const WHITE: u32 = 0xffffffff;
pub const GRAY: u32 = 0xa0a0a0ff;
pub const YELLOW: u32 = 0xffffa0ff;
pub const MOJANG_RED: u32 = 0xef323dff;

/// Màn hình Mojang: nền đỏ, logo hiện dần, thanh tiến trình thật. Trả về độ mờ nền còn lại (1 → 0 khi kết thúc).
pub struct Boot {
    pub t: f32,
    pub progress: f32,
    /// thời điểm bắt đầu mờ dần (None khi chưa xong)
    pub fade_start: Option<f32>,
}

impl Boot {
    pub fn new() -> Boot {
        Boot { t: 0.0, progress: 0.0, fade_start: None }
    }

    /// `real` là tiến độ nạp thật 0..1; mờ dần khi đã đầy và đã hiện đủ lâu.
    pub fn update(&mut self, dt: f32, real: f32, done: bool) {
        self.t += dt;
        // làm mượt như LoadingOverlay: progress = progress*0.95 + real*0.05 mỗi khung hình (quy về 60 khung/giây)
        let frames = (dt * 60.0).round().max(1.0) as i32;
        for _ in 0..frames {
            self.progress = (self.progress * 0.95 + real * 0.05).clamp(0.0, 1.0);
        }
        if done && self.progress > 0.98 && self.fade_start.is_none() && self.t > 1.2 {
            self.fade_start = Some(self.t);
        }
    }

    /// độ mờ nền: 1 đặc, 0 trong suốt
    pub fn alpha(&self) -> f32 {
        match self.fade_start {
            Some(s) => (1.0 - (self.t - s) / 1.0).clamp(0.0, 1.0),
            None => 1.0,
        }
    }

    pub fn finished(&self) -> bool {
        self.alpha() <= 0.0
    }

    pub fn draw(&self, d: &mut Draw) {
        let (w, h) = d.size();
        let a = self.alpha();
        d.fill(0.0, 0.0, w, h, with_alpha(MOJANG_RED, a));
        let logo_a = ((self.t / 0.5).clamp(0.0, 1.0)) * a.min(1.0);
        let Some(tex) = d.a.mojang else { return };
        let (i, j) = ((w * 0.5).floor(), (h * 0.5).floor());
        let dd = (w * 0.75).min(h) * 0.25;
        let k = (dd * 0.5).floor();
        let l = (dd * 4.0 * 0.5).floor();
        let col = with_alpha(WHITE, logo_a);
        // hai nửa logo xếp dọc trong ảnh: nửa trái ở trên, nửa phải ở dưới
        d.blit_logical(tex, [i - l, j - k, l, dd.floor()], [0.0, 0.0, 120.0, 60.0], 120.0, 120.0, col);
        d.blit_logical(tex, [i, j - k, l, dd.floor()], [0.0, 60.0, 120.0, 60.0], 120.0, 120.0, col);
        // thanh tiến trình
        let m = (h * 0.8325).floor();
        let (x0, y0, x1, y1) = (i - l, m - 5.0, i + l, m + 5.0);
        let n = ((x1 - x0 - 2.0) * self.progress).ceil();
        let bar = with_alpha(WHITE, a);
        d.fill(x0 + 2.0, y0 + 2.0, n.max(0.0), y1 - y0 - 4.0, bar);
        d.fill(x0 + 1.0, y0, x1 - x0 - 2.0, 1.0, bar);
        d.fill(x0 + 1.0, y1 - 1.0, x1 - x0 - 2.0, 1.0, bar);
        d.fill(x0, y0, 1.0, y1 - y0, bar);
        d.fill(x1 - 1.0, y0, 1.0, y1 - y0, bar);
    }
}

/// Nút bấm kiểu vanilla. Trả về true khi vừa được bấm.
pub fn button(d: &mut Draw, p: &Ptr, r: [f32; 4], label: &str, active: bool) -> bool {
    let hot = active && inside(r, p.down);
    let sprite = if !active {
        "widget/button_disabled"
    } else if hot {
        "widget/button_highlighted"
    } else {
        "widget/button"
    };
    d.sprite(sprite, r[0], r[1], r[2], r[3]);
    let col = if !active { GRAY } else if hot { YELLOW } else { WHITE };
    d.text_centered(label, r[0] + r[2] / 2.0, r[1] + (r[3] - 8.0) / 2.0, col);
    active && inside(r, p.tap)
}

/// Nút vuông có icon (ngôn ngữ, trợ năng...).
pub fn icon_button(d: &mut Draw, p: &Ptr, r: [f32; 4], icon: &str) -> bool {
    let hot = inside(r, p.down);
    d.sprite(if hot { "widget/button_highlighted" } else { "widget/button" }, r[0], r[1], r[2], r[3]);
    if let Some(s) = d.a.sprites.get(icon).copied() {
        let (iw, ih) = (s.w, s.h);
        d.blit(s.tex, [r[0] + (r[2] - iw) / 2.0, r[1] + (r[3] - ih) / 2.0, iw, ih], [0.0, 0.0, iw, ih], WHITE);
    }
    inside(r, p.tap)
}

pub enum TitleAction {
    Singleplayer,
    Multiplayer,
    Realms,
    Options,
    Quit,
    Language,
    Accessibility,
}

pub struct Title {
    pub t: f32,
    pub splash: String,
    /// độ hiện dần của các thành phần (vanilla: 1 giây)
    pub fade: f32,
}

impl Title {
    pub fn new(splashes: &[String], seed: u64) -> Title {
        let s = if splashes.is_empty() { String::new() } else { splashes[(seed % splashes.len() as u64) as usize].clone() };
        Title { t: 0.0, splash: s, fade: 0.0 }
    }

    pub fn draw(&mut self, d: &mut Draw, p: &Ptr, dt: f32, version: &str) -> Option<TitleAction> {
        self.t += dt;
        self.fade = (self.fade + dt).min(2.0);
        let (w, h) = d.size();
        panorama_bg(d, self.t, 0.0);
        // logo
        let ui_a = (self.fade / 1.0).clamp(0.0, 1.0);
        let wa = with_alpha(WHITE, ui_a);
        let cx = (w / 2.0).floor();
        if let Some(t) = d.a.logo {
            d.blit_logical(t, [cx - 128.0, 30.0, 256.0, 44.0], [0.0, 0.0, 256.0, 44.0], 256.0, 64.0, wa);
        }
        if let Some(t) = d.a.edition {
            d.blit_logical(t, [cx - 64.0, 30.0 + 44.0 - 7.0, 128.0, 14.0], [0.0, 0.0, 128.0, 14.0], 128.0, 16.0, wa);
        }
        // splash: xoay -20°, nhấp nháy theo thời gian
        if !self.splash.is_empty() {
            let ms = (self.t * 1000.0) as u64;
            let mut f = 1.8 - ((ms % 1000) as f32 / 1000.0 * std::f32::consts::TAU).sin().abs() * 0.1;
            f = f * 100.0 / (d.text_width(&self.splash) + 32.0);
            let ang = -20f32.to_radians();
            let (sn, cs) = ang.sin_cos();
            let k = d.k;
            // dịch tới (w/2+123, 69), xoay, co theo f; vẽ chữ căn giữa tại (0,-8)
            let xf = Xf { a: cs * f * k, b: sn * f * k, c: -sn * f * k, d: cs * f * k, tx: (cx + 123.0) * k, ty: 69.0 * k };
            d.c.set_xf(xf);
            let saved = d.k;
            d.k = 1.0;
            let sw = d.text_width(&self.splash);
            d.text_raw(&self.splash, -sw / 2.0 + 1.0, -8.0 + 1.0, scale_rgb(with_alpha(0xffff00ff, ui_a), 0.25));
            d.text_raw(&self.splash, -sw / 2.0, -8.0, with_alpha(0xffff00ff, ui_a));
            d.k = saved;
            d.c.set_xf(Xf::ID);
        }
        // nút
        let mut act = None;
        let i = (h / 4.0).floor() + 48.0;
        let cxl = cx - 100.0;
        let sp = d.a.tr("menu.singleplayer");
        let mp = d.a.tr("menu.multiplayer");
        let on = d.a.tr("menu.online");
        let op = d.a.tr("menu.options");
        let qt = d.a.tr("menu.quit");
        if button(d, p, [cxl, i, 200.0, 20.0], &sp, true) {
            act = Some(TitleAction::Singleplayer);
        }
        if button(d, p, [cxl, i + 24.0, 200.0, 20.0], &mp, true) {
            act = Some(TitleAction::Multiplayer);
        }
        if button(d, p, [cxl, i + 48.0, 200.0, 20.0], &on, true) {
            act = Some(TitleAction::Realms);
        }
        if icon_button(d, p, [cx - 124.0, i + 72.0 + 12.0, 20.0, 20.0], "icon/language") {
            act = Some(TitleAction::Language);
        }
        if button(d, p, [cx - 100.0, i + 72.0 + 12.0, 98.0, 20.0], &op, true) {
            act = Some(TitleAction::Options);
        }
        if button(d, p, [cx + 2.0, i + 72.0 + 12.0, 98.0, 20.0], &qt, true) {
            act = Some(TitleAction::Quit);
        }
        if icon_button(d, p, [cx + 104.0, i + 72.0 + 12.0, 20.0, 20.0], "icon/accessibility") {
            act = Some(TitleAction::Accessibility);
        }
        // chữ góc dưới
        let ver = format!("Minecraft {version}");
        d.text(&ver, 2.0, h - 10.0, wa);
        let credit = d.a.tr("title.credits");
        let cw = d.text_width(&credit);
        d.text(&credit, w - cw - 2.0, h - 10.0, wa);
        act
    }
}

/// Nền panorama xoay chậm (như vanilla: 0.1°/tick, cúi 25° dao động nhẹ).
pub fn panorama_bg(d: &mut Draw, t: f32, blur: f32) {
    if let Some(f) = d.a.pano {
        let spin = t * 20.0;
        let yaw = spin * 0.1;
        let pitch = (spin * 0.001).sin() * 5.0 + 25.0;
        d.c.panorama(&f, yaw, pitch, 85.0, 1.0, blur * d.k);
    } else {
        let (w, h) = d.size();
        d.fill(0.0, 0.0, w, h, 0x202428ff);
    }
}
