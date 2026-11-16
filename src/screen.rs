//! Màn hình launcher vẽ tay bằng Gfx: danh sách phiên bản, nút chạy, tiến trình, nhật ký. Điều khiển bằng chạm/kéo.
use std::sync::atomic::Ordering::Relaxed;

use crate::{gfx::Gfx, launcher::Launcher};

const BG: u32 = 0x1c1c1eff;
const PANEL: u32 = 0x262628ff;
const TEXT: u32 = 0xe8e8e8ff;
const DIM: u32 = 0x9a9aa0ff;
const BTN: u32 = 0x2f6fb3ff;
const BTN_OFF: u32 = 0x3a3a3cff;
const SEL: u32 = 0x2d5a8aff;

#[derive(Clone, Copy, PartialEq)]
enum Region {
    None,
    Versions,
    Log,
}

#[derive(Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn has(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }
}

pub struct Screen {
    show_snapshots: bool,
    selected: Option<String>,
    // cảm ứng
    press: Option<(f32, f32)>,
    last: (f32, f32),
    dragging: bool,
    drag_region: Region,
    tap: Option<(f32, f32)>,
    // cuộn
    ver_scroll: f32,
    log_scroll: f32,
    log_stick: bool,
    ver_rect: Rect,
    log_rect: Rect,
    // nhật ký đã ngắt dòng (cache)
    wrapped: Vec<String>,
    wrapped_from: usize,
    wrapped_w: f32,
    wrapped_size: f32,
}

impl Screen {
    pub fn new() -> Self {
        let z = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        Self {
            show_snapshots: false,
            selected: None,
            press: None,
            last: (0.0, 0.0),
            dragging: false,
            drag_region: Region::None,
            tap: None,
            ver_scroll: 0.0,
            log_scroll: 0.0,
            log_stick: true,
            ver_rect: z,
            log_rect: z,
            wrapped: Vec::new(),
            wrapped_from: 0,
            wrapped_w: 0.0,
            wrapped_size: 0.0,
        }
    }

    pub fn touch_down(&mut self, x: f32, y: f32, scale: f32) {
        let _ = scale;
        self.press = Some((x, y));
        self.last = (x, y);
        self.dragging = false;
        self.drag_region = if self.ver_rect.has((x, y)) {
            Region::Versions
        } else if self.log_rect.has((x, y)) {
            Region::Log
        } else {
            Region::None
        };
    }

    pub fn touch_move(&mut self, x: f32, y: f32, scale: f32) {
        let Some(p) = self.press else { return };
        if !self.dragging && ((x - p.0).powi(2) + (y - p.1).powi(2)).sqrt() > 10.0 * scale {
            self.dragging = true;
        }
        if self.dragging {
            let dy = y - self.last.1;
            match self.drag_region {
                Region::Versions => self.ver_scroll -= dy,
                Region::Log => {
                    self.log_scroll -= dy;
                    self.log_stick = false;
                }
                Region::None => {}
            }
        }
        self.last = (x, y);
    }

    pub fn touch_up(&mut self, x: f32, y: f32) {
        if self.press.is_some() && !self.dragging {
            self.tap = Some((x, y));
        }
        self.press = None;
        self.dragging = false;
    }

    fn update_wrap(&mut self, g: &mut Gfx, l: &Launcher, size: f32, max_w: f32) {
        let log = l.log.lock().unwrap();
        if (self.wrapped_w - max_w).abs() > 0.5 || (self.wrapped_size - size).abs() > 0.01 || log.len() < self.wrapped_from {
            self.wrapped.clear();
            self.wrapped_from = 0;
            self.wrapped_w = max_w;
            self.wrapped_size = size;
        }
        for line in &log[self.wrapped_from..] {
            let parts = g.wrap(line, size, max_w);
            self.wrapped.extend(parts);
        }
        self.wrapped_from = log.len();
        if self.wrapped.len() > 6000 {
            let cut = self.wrapped.len() - 5000;
            self.wrapped.drain(..cut);
        }
    }

    /// Vẽ một khung hình. Trả về phiên bản vừa được bấm "chạy" (nếu có).
    pub fn draw(&mut self, g: &mut Gfx, l: &Launcher) -> Option<String> {
        let s = g.scale;
        let (w, h) = (g.w as f32, g.h as f32);
        let m = 16.0 * s;
        let tap = self.tap.take();
        let mut run: Option<String> = None;
        g.begin();

        let mut y = 40.0 * s;
        g.text(m, y, 26.0 * s, TEXT, "Rust Launcher");
        y += 34.0 * s;
        g.text(m, y, 12.0 * s, DIM, "Mốc 4b-1: giao diện tự vẽ (EGL + GLES)");
        y += 22.0 * s;

        // checkbox snapshot
        let cb = 20.0 * s;
        let row = Rect { x: m, y, w: w - 2.0 * m, h: cb + 8.0 * s };
        if let Some(t) = tap {
            if row.has(t) {
                self.show_snapshots = !self.show_snapshots;
            }
        }
        g.rect(m, y + 4.0 * s, cb, cb, if self.show_snapshots { SEL } else { PANEL });
        if self.show_snapshots {
            g.rect(m + 5.0 * s, y + 9.0 * s, cb - 10.0 * s, cb - 10.0 * s, TEXT);
        }
        g.text(m + cb + 10.0 * s, y + 6.0 * s, 14.0 * s, TEXT, "Hiện cả snapshot");
        y += cb + 14.0 * s;

        // danh sách phiên bản
        let item_h = 36.0 * s;
        let list = Rect { x: m, y, w: w - 2.0 * m, h: 150.0 * s };
        self.ver_rect = list;
        let ids: Vec<String> = l
            .shared
            .versions
            .lock()
            .unwrap()
            .iter()
            .filter(|v| self.show_snapshots || v.kind == "release")
            .map(|v| v.id.clone())
            .collect();
        let content = ids.len() as f32 * item_h;
        self.ver_scroll = self.ver_scroll.clamp(0.0, (content - list.h).max(0.0));
        g.rect(list.x, list.y, list.w, list.h, PANEL);
        g.set_clip(list.x, list.y, list.w, list.h);
        for (i, id) in ids.iter().enumerate() {
            let iy = list.y + i as f32 * item_h - self.ver_scroll;
            if iy + item_h < list.y || iy > list.y + list.h {
                continue;
            }
            let r = Rect { x: list.x, y: iy, w: list.w, h: item_h };
            if self.selected.as_deref() == Some(id.as_str()) {
                g.rect(r.x, r.y, r.w, r.h, SEL);
            }
            if let Some(t) = tap {
                if list.has(t) && r.has(t) {
                    self.selected = Some(id.clone());
                }
            }
            g.text(r.x + 10.0 * s, r.y + 8.0 * s, 16.0 * s, TEXT, id);
        }
        g.clear_clip();
        y += list.h + 12.0 * s;

        // nút chạy
        let busy = l.busy.load(Relaxed);
        let used = l.jvm_used.load(Relaxed);
        let can = !busy && !used && self.selected.is_some();
        let label = match (&self.selected, used) {
            (_, true) => "JVM đã chạy (mở lại app để chạy lần nữa)".to_string(),
            (Some(v), _) => format!("Cài và chạy {v}"),
            (None, _) => "Chọn một phiên bản".to_string(),
        };
        let btn = Rect { x: m, y, w: w - 2.0 * m, h: 46.0 * s };
        g.rect(btn.x, btn.y, btn.w, btn.h, if can { BTN } else { BTN_OFF });
        let tw = g.text_width(&label, 16.0 * s);
        g.text(btn.x + (btn.w - tw) / 2.0, btn.y + 12.0 * s, 16.0 * s, if can { 0xffffffff } else { DIM }, &label);
        if can {
            if let Some(t) = tap {
                if btn.has(t) {
                    run = self.selected.clone();
                }
            }
        }
        y += btn.h + 12.0 * s;

        // trạng thái + tiến trình
        let phase = l.shared.phase.lock().unwrap().clone();
        g.text(m, y, 14.0 * s, TEXT, &phase);
        y += 22.0 * s;
        let (d, t) = (l.shared.bytes_done.load(Relaxed), l.shared.bytes_total.load(Relaxed));
        if t > 0 {
            let bar = Rect { x: m, y, w: w - 2.0 * m, h: 16.0 * s };
            g.rect(bar.x, bar.y, bar.w, bar.h, PANEL);
            g.rect(bar.x, bar.y, bar.w * (d as f32 / t as f32).min(1.0), bar.h, BTN);
            let txt = format!("{} / {} MB", d / 1_000_000, t / 1_000_000);
            g.text(bar.x + 6.0 * s, bar.y + 1.0 * s, 11.0 * s, 0xffffffff, &txt);
            y += bar.h + 6.0 * s;
        }
        let (fd, ft) = (l.shared.files_done.load(Relaxed), l.shared.files_total.load(Relaxed));
        if ft > 0 {
            g.text(m, y, 12.0 * s, DIM, &format!("{fd} / {ft} file"));
            y += 18.0 * s;
        } else if fd > 0 {
            g.text(m, y, 12.0 * s, DIM, &format!("{fd} mục đã giải nén"));
            y += 18.0 * s;
        }
        y += 6.0 * s;

        // nhật ký
        let font = 11.0 * s;
        let lh = g.line_height(font);
        let area = Rect { x: m, y, w: w - 2.0 * m, h: (h - y - m).max(40.0 * s) };
        self.log_rect = area;
        self.update_wrap(g, l, font, area.w - 12.0 * s);
        let content = self.wrapped.len() as f32 * lh + 8.0 * s;
        let max_scroll = (content - area.h).max(0.0);
        if self.log_stick {
            self.log_scroll = max_scroll;
        } else if self.log_scroll >= max_scroll - 2.0 {
            self.log_stick = true;
        }
        self.log_scroll = self.log_scroll.clamp(0.0, max_scroll);
        g.rect(area.x, area.y, area.w, area.h, 0x141416ff);
        g.set_clip(area.x, area.y, area.w, area.h);
        let first = (self.log_scroll / lh).floor().max(0.0) as usize;
        let last = (((self.log_scroll + area.h) / lh).ceil() as usize + 1).min(self.wrapped.len());
        for i in first..last {
            let ly = area.y + 4.0 * s + i as f32 * lh - self.log_scroll;
            g.text(area.x + 6.0 * s, ly, font, 0xd0d0d0ff, &self.wrapped[i]);
        }
        g.clear_clip();

        if !g.has_font() {
            g.rect(0.0, 0.0, w, 6.0, 0xff0000ff);
        }
        g.end(BG);
        run
    }
}
