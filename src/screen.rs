//! Màn hình launcher: thanh tài khoản, menu, thanh phiên bản + PLAY. Vẽ tay bằng Gfx, điều khiển bằng chạm/kéo.
use std::sync::atomic::Ordering::Relaxed;

use crate::{
    account::Accounts,
    auth::{self, Shared, Status},
    gfx::Gfx,
    keyboard::{Kb, Keyboard},
    launcher::Launcher,
    ui::*,
};

pub enum Action {
    View(String),
    World(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Main,
    Versions,
    Accounts,
    AddName,
    Ely,
    Ms,
    Log,
}

pub struct Screen {
    page: Page,
    show_snapshots: bool,
    selected: Option<String>,
    accounts: Option<Accounts>,
    kb: Keyboard,
    name_buf: String,
    name_err: Option<&'static str>,
    hint: Option<&'static str>,
    auth: Shared,
    fields: [String; 3],
    focus: usize,
    need_totp: bool,
    ms_started: bool,
    press: Option<(f32, f32)>,
    last: (f32, f32),
    dragging: bool,
    tap: Option<(f32, f32)>,
    scroll: f32,
    log_scroll: f32,
    log_stick: bool,
    wrapped: Vec<String>,
    wrapped_from: usize,
    wrapped_w: f32,
    wrapped_size: f32,
}

impl Screen {
    pub fn new() -> Self {
        Self {
            page: Page::Main,
            show_snapshots: false,
            selected: None,
            accounts: None,
            kb: Keyboard::new(),
            name_buf: String::new(),
            name_err: None,
            hint: None,
            auth: auth::new_shared(),
            fields: Default::default(),
            focus: 0,
            need_totp: false,
            ms_started: false,
            press: None,
            last: (0.0, 0.0),
            dragging: false,
            tap: None,
            scroll: 0.0,
            log_scroll: 0.0,
            log_stick: true,
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
    }

    pub fn touch_move(&mut self, x: f32, y: f32, scale: f32) {
        let Some(p) = self.press else { return };
        if !self.dragging && ((x - p.0).powi(2) + (y - p.1).powi(2)).sqrt() > 10.0 * scale {
            self.dragging = true;
        }
        if self.dragging {
            let dy = y - self.last.1;
            match self.page {
                Page::Versions | Page::Accounts => self.scroll -= dy,
                Page::Log => {
                    self.log_scroll -= dy;
                    self.log_stick = false;
                }
                _ => {}
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

    fn go(&mut self, p: Page) {
        self.page = p;
        self.scroll = 0.0;
        if p == Page::Main {
            self.hint = None;
        }
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

    /// Thanh tiêu đề của trang con. Trả về (đã chạm nút quay lại, y đáy thanh).
    fn header(&mut self, g: &mut Gfx, title: &str, tap: Option<(f32, f32)>) -> (bool, f32) {
        let s = g.scale;
        let w = g.w as f32;
        let (inset, bh) = (30.0 * s, 56.0 * s);
        g.rect(0.0, 0.0, w, inset + bh, BAR);
        g.rect(0.0, inset + bh, w, 2.0 * s, ACCENT);
        icon(g, &ICON_BACK, 20.0 * s, inset + (bh - 24.0 * s) / 2.0, 3.0 * s, TEXT);
        text_v(g, 64.0 * s, Rect::new(0.0, inset, w, bh), 18.0 * s, TEXT, title);
        (hit(Rect::new(0.0, inset, 64.0 * s, bh), tap), inset + bh + 2.0 * s)
    }

    /// Vẽ một khung hình. Trả về hành động khi người dùng bấm chạy.
    pub fn draw(&mut self, g: &mut Gfx, l: &Launcher) -> Option<Action> {
        if self.accounts.is_none() {
            self.accounts = Some(Accounts::load(&l.root));
        }
        let tap = self.tap.take();
        let done = {
            let mut st = self.auth.lock().unwrap();
            if let Status::Done(a) = &*st {
                let a = a.clone();
                *st = Status::Idle;
                Some(a)
            } else {
                None
            }
        };
        if let Some(a) = done {
            if let Some(acc) = self.accounts.as_mut() {
                acc.add(a);
            }
            self.fields = Default::default();
            self.need_totp = false;
            self.go(Page::Main);
        }
        g.begin();
        let mut act = None;
        match self.page {
            Page::Main => act = self.page_main(g, l, tap),
            Page::Versions => self.page_versions(g, l, tap),
            Page::Accounts => self.page_accounts(g, tap),
            Page::AddName => self.page_addname(g, tap),
            Page::Ely => self.page_ely(g, tap),
            Page::Ms => self.page_ms(g, l, tap),
            Page::Log => self.page_log(g, l, tap),
        }
        if !g.has_font() {
            g.rect(0.0, 0.0, g.w as f32, 6.0, 0xff0000ff);
        }
        g.end(Some(BG));
        act
    }

    fn page_main(&mut self, g: &mut Gfx, l: &Launcher, tap: Option<(f32, f32)>) -> Option<Action> {
        let s = g.scale;
        let (w, h) = (g.w as f32, g.h as f32);
        let m = 16.0 * s;
        let (inset, tb) = (30.0 * s, 64.0 * s);
        let mut nav: Option<Page> = None;
        let mut run: Option<Action> = None;

        // chọn sẵn một phiên bản (ưu tiên 1.21.10) để PLAY dùng được ngay
        if self.selected.is_none() {
            let pick: Option<String>;
            {
                let vs = l.shared.versions.lock().unwrap();
                pick = vs.iter().find(|v| v.id == "1.21.10").or_else(|| vs.iter().find(|v| v.kind == "release")).map(|v| v.id.clone());
            }
            self.selected = pick;
        }
        let acc_name = self.accounts.as_ref().and_then(|a| a.current()).map(|a| a.name.clone());

        // thanh trên: tài khoản + nhật ký
        g.rect(0.0, 0.0, w, inset + tb, BAR);
        g.rect(0.0, inset + tb, w, 2.0 * s, ACCENT);
        let av = Rect::new(m, inset + (tb - 40.0 * s) / 2.0, 40.0 * s, 40.0 * s);
        let bar = Rect::new(0.0, inset, w, tb);
        match &acc_name {
            Some(n) => {
                avatar(g, av, n);
                text_v(g, m + 52.0 * s, bar, 18.0 * s, TEXT, n);
            }
            None => {
                g.rect(av.x, av.y, av.w, av.h, OFF);
                icon(g, &ICON_USER, av.x + 8.0 * s, av.y + 8.0 * s, 3.0 * s, DIM);
                text_v(g, m + 52.0 * s, bar, 16.0 * s, DIM, "Chưa có tài khoản");
            }
        }
        icon(g, &ICON_LOG, w - 48.0 * s, inset + (tb - 24.0 * s) / 2.0, 3.0 * s, DIM);
        if hit(Rect::new(w - 76.0 * s, inset, 76.0 * s, tb), tap) {
            nav = Some(Page::Log);
        } else if hit(Rect::new(0.0, inset, w - 76.0 * s, tb), tap) {
            nav = Some(Page::Accounts);
        }

        // phần đầu trang
        let hy = inset + tb + 2.0 * s;
        let hh = 180.0 * s;
        vgrad(g, Rect::new(0.0, hy, w, hh), 0x0f2b31ff, BG);
        let l40 = g.line_height(40.0 * s);
        g.text(m, hy + 30.0 * s, 40.0 * s, ACCENT, "RUST");
        g.text(m, hy + 30.0 * s + l40, 40.0 * s, TEXT, "LAUNCHER");

        // menu
        let items: [(&[u8; 8], &str); 3] = [(&ICON_USER, "TÀI KHOẢN"), (&ICON_CUBE, "XEM KHỐI MẪU"), (&ICON_LOG, "NHẬT KÝ")];
        let rh = 64.0 * s;
        let mut y = hy + hh + 4.0 * s;
        let busy = l.busy.load(Relaxed);
        for (i, (ic, label)) in items.iter().enumerate() {
            let r = Rect::new(0.0, y, w, rh);
            icon(g, ic, m + 4.0 * s, y + (rh - 32.0 * s) / 2.0, 4.0 * s, ACCENT);
            text_v(g, m + 64.0 * s, r, 18.0 * s, TEXT, label);
            g.rect(m, y + rh - 1.0, w - 2.0 * m, 1.0, LINE);
            if hit(r, tap) {
                match i {
                    0 => nav = Some(Page::Accounts),
                    1 => {
                        if !busy {
                            if let Some(v) = &self.selected {
                                run = Some(Action::View(v.clone()));
                            }
                        }
                    }
                    _ => nav = Some(Page::Log),
                }
            }
            y += rh;
        }

        // thanh dưới: tiến trình, phiên bản, PLAY
        let bar_top = h - 36.0 * s - 150.0 * s;
        g.rect(0.0, bar_top, w, h - bar_top, BAR);
        g.rect(0.0, bar_top, w, 1.0, LINE);
        let mut info = l.shared.phase.lock().unwrap().clone();
        let (d, t) = (l.shared.bytes_done.load(Relaxed), l.shared.bytes_total.load(Relaxed));
        let (fd, ft) = (l.shared.files_done.load(Relaxed), l.shared.files_total.load(Relaxed));
        if ft > 0 {
            info = format!("{info}  ·  {fd}/{ft} file");
        }
        if busy || t > 0 {
            g.text(m, bar_top - 44.0 * s, 12.0 * s, DIM, &info);
        }
        if t > 0 {
            let by = bar_top - 16.0 * s;
            g.rect(m, by, w - 2.0 * m, 5.0 * s, OFF);
            g.rect(m, by, (w - 2.0 * m) * (d as f32 / t as f32).min(1.0), 5.0 * s, ACCENT);
        }

        let card = Rect::new(m, bar_top + 14.0 * s, w - 2.0 * m, 56.0 * s);
        g.rect(card.x, card.y, card.w, card.h, CARD);
        let tile = Rect::new(card.x + 8.0 * s, card.y + 8.0 * s, 40.0 * s, 40.0 * s);
        vgrad(g, tile, ACCENT, ACCENT_DK);
        text_c(g, tile, 16.0 * s, 0x022b25ff, "MC");
        let vlabel = self.selected.clone().unwrap_or_else(|| "Chọn phiên bản".to_string());
        text_v(g, card.x + 60.0 * s, card, 18.0 * s, TEXT, &vlabel);
        icon(g, &ICON_UP, card.x + card.w - 40.0 * s, card.y + (card.h - 24.0 * s) / 2.0, 3.0 * s, DIM);
        if hit(card, tap) {
            nav = Some(Page::Versions);
        }

        let play = Rect::new(m, card.y + card.h + 10.0 * s, w - 2.0 * m, 52.0 * s);
        let can = !busy && self.selected.is_some();
        if can {
            vgrad(g, play, ACCENT, ACCENT_DK);
            g.rect(play.x, play.y, play.w, 2.0 * s, 0x9ff7ebff);
            text_c(g, play, 20.0 * s, 0x022b25ff, "PLAY");
        } else {
            g.rect(play.x, play.y, play.w, play.h, OFF);
            text_c(g, play, 16.0 * s, DIM, if busy { "ĐANG CHUẨN BỊ..." } else { "CHỌN PHIÊN BẢN" });
        }
        if can && hit(play, tap) {
            if self.accounts.as_ref().map_or(true, |a| a.current().is_none()) {
                nav = Some(Page::Accounts);
                self.hint = Some("Thêm một tài khoản (ngoại tuyến) rồi quay lại bấm PLAY");
            } else {
                run = self.selected.clone().map(Action::World);
            }
        }

        if let Some(p) = nav {
            self.go(p);
        }
        run
    }

    fn page_versions(&mut self, g: &mut Gfx, l: &Launcher, tap: Option<(f32, f32)>) {
        let (back, top) = self.header(g, "Phiên bản", tap);
        if back {
            self.go(Page::Main);
            return;
        }
        let s = g.scale;
        let (w, h) = (g.w as f32, g.h as f32);
        let m = 16.0 * s;
        let snaps = self.show_snapshots;
        let row = Rect::new(0.0, top, w, 56.0 * s);
        let bx = Rect::new(m, top + 16.0 * s, 24.0 * s, 24.0 * s);
        g.rect(bx.x, bx.y, bx.w, bx.h, if snaps { ACCENT_DK } else { CARD_HI });
        if snaps {
            g.rect(bx.x + 6.0 * s, bx.y + 6.0 * s, 12.0 * s, 12.0 * s, TEXT);
        }
        text_v(g, m + 36.0 * s, row, 15.0 * s, TEXT, "Hiện cả snapshot");
        g.rect(0.0, row.y + row.h - 1.0, w, 1.0, LINE);
        if hit(row, tap) {
            self.show_snapshots = !snaps;
            self.scroll = 0.0;
        }

        let list = Rect::new(0.0, top + 56.0 * s, w, h - (top + 56.0 * s) - 36.0 * s);
        let ids: Vec<(String, String)> = l
            .shared
            .versions
            .lock()
            .unwrap()
            .iter()
            .filter(|v| snaps || v.kind == "release")
            .map(|v| (v.id.clone(), v.kind.clone()))
            .collect();
        let ih = 60.0 * s;
        self.scroll = self.scroll.clamp(0.0, (ids.len() as f32 * ih - list.h).max(0.0));
        let in_list = tap.map_or(false, |t| list.has(t));
        let mut chosen: Option<String> = None;
        g.set_clip(list.x, list.y, list.w, list.h);
        for (i, (id, kind)) in ids.iter().enumerate() {
            let y = list.y + i as f32 * ih - self.scroll;
            if y + ih < list.y || y > list.y + list.h {
                continue;
            }
            let r = Rect::new(0.0, y, w, ih);
            if self.selected.as_deref() == Some(id.as_str()) {
                g.rect(r.x, r.y, r.w, r.h, CARD_HI);
                g.rect(r.x, r.y, 4.0 * s, r.h, ACCENT);
            }
            text_v(g, m + 8.0 * s, r, 17.0 * s, TEXT, id);
            let kw = g.text_width(kind, 12.0 * s);
            text_v(g, w - m - kw, r, 12.0 * s, DIM, kind);
            g.rect(m, y + ih - 1.0, w - 2.0 * m, 1.0, LINE);
            if in_list && hit(r, tap) {
                chosen = Some(id.clone());
            }
        }
        g.clear_clip();
        if let Some(id) = chosen {
            self.selected = Some(id);
            self.go(Page::Main);
        }
    }

    fn page_accounts(&mut self, g: &mut Gfx, tap: Option<(f32, f32)>) {
        let (back, top) = self.header(g, "Tài khoản", tap);
        if back {
            self.go(Page::Main);
            return;
        }
        let s = g.scale;
        let (w, h) = (g.w as f32, g.h as f32);
        let m = 16.0 * s;
        let (bh, gap) = (52.0 * s, 10.0 * s);
        let block_top = h - 36.0 * s - (3.0 * bh + 4.0 * gap);
        let mut ly = top + 8.0 * s;
        if let Some(t) = self.hint {
            g.text(m, ly + 4.0 * s, 13.0 * s, WARN, t);
            ly += 30.0 * s;
        }
        let list = Rect::new(0.0, ly, w, block_top - ly);
        let (rows, sel): (Vec<(String, &'static str)>, usize) = match &self.accounts {
            Some(a) => (
                a.list
                    .iter()
                    .map(|x| (x.name.clone(), x.kind.label()))
                    .collect(),
                a.sel,
            ),
            None => (Vec::new(), 0),
        };
        let ih = 72.0 * s;
        let l17 = g.line_height(17.0 * s);
        self.scroll = self.scroll.clamp(0.0, (rows.len() as f32 * ih - list.h).max(0.0));
        if rows.is_empty() {
            text_c(g, list, 14.0 * s, DIM, "Chưa có tài khoản nào");
        }
        let in_list = tap.map_or(false, |t| list.has(t));
        let (mut del, mut pick): (Option<usize>, Option<usize>) = (None, None);
        g.set_clip(list.x, list.y, list.w, list.h);
        for (i, (name, label)) in rows.iter().enumerate() {
            let y = list.y + i as f32 * ih - self.scroll;
            if y + ih < list.y || y > list.y + list.h {
                continue;
            }
            let r = Rect::new(0.0, y, w, ih);
            if i == sel {
                g.rect(r.x, r.y, r.w, r.h, CARD_HI);
                g.rect(r.x, r.y, 4.0 * s, r.h, ACCENT);
                icon(g, &ICON_CHECK, w - 100.0 * s, y + (ih - 24.0 * s) / 2.0, 3.0 * s, ACCENT);
            }
            avatar(g, Rect::new(m + 4.0 * s, y + (ih - 44.0 * s) / 2.0, 44.0 * s, 44.0 * s), name);
            g.text(m + 64.0 * s, y + 14.0 * s, 17.0 * s, TEXT, name);
            g.text(m + 64.0 * s, y + 14.0 * s + l17 + 2.0 * s, 12.0 * s, DIM, label);
            icon(g, &ICON_TRASH, w - 48.0 * s, y + (ih - 24.0 * s) / 2.0, 3.0 * s, DIM);
            g.rect(m, y + ih - 1.0, w - 2.0 * m, 1.0, LINE);
            if in_list {
                if hit(Rect::new(w - 64.0 * s, y, 64.0 * s, ih), tap) {
                    del = Some(i);
                } else if hit(r, tap) {
                    pick = Some(i);
                }
            }
        }
        g.clear_clip();

        // nút thêm tài khoản
        let bx = |i: usize| Rect::new(m, block_top + gap + i as f32 * (bh + gap), w - 2.0 * m, bh);
        let r0 = bx(0);
        vgrad(g, r0, ACCENT, ACCENT_DK);
        text_c(g, r0, 16.0 * s, 0x022b25ff, "+  NGOẠI TUYẾN");
        let (r1, r2) = (bx(1), bx(2));
        for (r, name) in [(r1, "MICROSOFT"), (r2, "ELY.BY")] {
            g.rect(r.x, r.y, r.w, r.h, CARD_HI);
            text_c(g, r, 16.0 * s, TEXT, name);
        }

        if let Some(i) = del {
            if let Some(a) = self.accounts.as_mut() {
                a.remove(i);
            }
        } else if let Some(i) = pick {
            if let Some(a) = self.accounts.as_mut() {
                a.select(i);
            }
            self.go(Page::Main);
        } else if hit(r0, tap) {
            self.name_buf.clear();
            self.name_err = None;
            self.kb.reset();
            self.go(Page::AddName);
        } else if hit(r1, tap) {
            *self.auth.lock().unwrap() = Status::Idle;
            self.ms_started = false;
            self.go(Page::Ms);
        } else if hit(r2, tap) {
            *self.auth.lock().unwrap() = Status::Idle;
            self.fields = Default::default();
            self.focus = 0;
            self.need_totp = false;
            self.kb.reset();
            self.go(Page::Ely);
        }
    }

    fn page_addname(&mut self, g: &mut Gfx, tap: Option<(f32, f32)>) {
        let (back, top) = self.header(g, "Tài khoản ngoại tuyến", tap);
        if back {
            self.go(Page::Accounts);
            return;
        }
        let s = g.scale;
        let (w, h) = (g.w as f32, g.h as f32);
        let m = 16.0 * s;
        let bx = Rect::new(m, top + 24.0 * s, w - 2.0 * m, 60.0 * s);
        g.rect(bx.x, bx.y, bx.w, bx.h, CARD);
        g.rect(bx.x, bx.y + bx.h - 2.0 * s, bx.w, 2.0 * s, ACCENT);
        if self.name_buf.is_empty() {
            text_v(g, bx.x + 16.0 * s, bx, 20.0 * s, DIM, "Nhập tên người chơi");
        } else {
            let t = self.name_buf.clone();
            text_v(g, bx.x + 16.0 * s, bx, 20.0 * s, TEXT, &t);
        }
        let l12 = g.line_height(12.0 * s);
        g.text(m, bx.y + bx.h + 12.0 * s, 12.0 * s, DIM, "1-16 ký tự: chữ, số hoặc dấu _");
        if let Some(e) = self.name_err {
            g.text(m, bx.y + bx.h + 12.0 * s + l12 + 6.0 * s, 13.0 * s, WARN, e);
        }

        let kh = 300.0 * s;
        let area = Rect::new(0.0, h - 36.0 * s - kh, w, kh);
        match self.kb.draw(g, area, tap) {
            Some(Kb::Ch(c)) => {
                if self.name_buf.len() < 16 {
                    self.name_buf.push(c);
                }
                self.name_err = None;
            }
            Some(Kb::Back) => {
                self.name_buf.pop();
                self.name_err = None;
            }
            Some(Kb::Cancel) => self.go(Page::Accounts),
            Some(Kb::Done) => {
                let n = self.name_buf.clone();
                let r = match self.accounts.as_mut() {
                    Some(a) => a.add_offline(&n),
                    None => Err("Chưa nạp được danh sách tài khoản"),
                };
                match r {
                    Ok(()) => self.go(Page::Main),
                    Err(e) => self.name_err = Some(e),
                }
            }
            _ => {}
        }
    }

    fn page_log(&mut self, g: &mut Gfx, l: &Launcher, tap: Option<(f32, f32)>) {
        let (back, top) = self.header(g, "Nhật ký", tap);
        if back {
            self.go(Page::Main);
            return;
        }
        let s = g.scale;
        let (w, h) = (g.w as f32, g.h as f32);
        let m = 16.0 * s;
        let font = 11.0 * s;
        let lh = g.line_height(font);
        let area = Rect::new(m, top + 10.0 * s, w - 2.0 * m, h - top - 10.0 * s - 36.0 * s);
        self.update_wrap(g, l, font, area.w - 12.0 * s);
        g.rect(area.x, area.y, area.w, area.h, 0x0a0e13ff);
        let total = self.wrapped.len() as f32 * lh;
        let max_scroll = (total - (area.h - 8.0 * s)).max(0.0);
        if self.log_stick {
            self.log_scroll = max_scroll;
        }
        self.log_scroll = self.log_scroll.clamp(0.0, max_scroll);
        if self.log_scroll >= max_scroll - 1.0 {
            self.log_stick = true;
        }
        g.set_clip(area.x, area.y, area.w, area.h);
        let first = (self.log_scroll / lh).floor() as usize;
        let mut ly = area.y + 4.0 * s - (self.log_scroll - first as f32 * lh);
        for line in self.wrapped.iter().skip(first) {
            if ly > area.y + area.h {
                break;
            }
            g.text(area.x + 6.0 * s, ly, font, 0xc9d4e0ff, line);
            ly += lh;
        }
        g.clear_clip();
    }

    /// Vẽ đoạn chữ tự xuống dòng, trả về y sau dòng cuối.
    fn para(&self, g: &mut Gfx, x: f32, y: f32, w: f32, size: f32, color: u32, text: &str) -> f32 {
        let lh = g.line_height(size);
        let mut y = y;
        for line in g.wrap(text, size, w) {
            g.text(x, y, size, color, &line);
            y += lh;
        }
        y
    }

    fn page_ms(&mut self, g: &mut Gfx, l: &Launcher, tap: Option<(f32, f32)>) {
        let (back, top) = self.header(g, "Đăng nhập Microsoft", tap);
        if back {
            *self.auth.lock().unwrap() = Status::Idle;
            self.go(Page::Accounts);
            return;
        }
        let path = l.root.join("ms_client_id.txt");
        if !self.ms_started {
            self.ms_started = true;
            match std::fs::read_to_string(&path) {
                Ok(t) if !t.trim().is_empty() => auth::ms_login(self.auth.clone(), t.trim().to_string()),
                _ => {
                    let msg = format!("Chưa có Client ID. Tạo file {} chỉ chứa Client ID của ứng dụng Azure đã được Mojang duyệt.", path.display());
                    *self.auth.lock().unwrap() = Status::Failed(msg);
                }
            }
        }
        let st = self.auth.lock().unwrap().clone();
        let s = g.scale;
        let (w, h) = (g.w as f32, g.h as f32);
        let m = 16.0 * s;
        let mut y = top + 28.0 * s;
        let mut retry = false;
        match st {
            Status::Code { code, url } => {
                y = self.para(g, m, y, w - 2.0 * m, 15.0 * s, DIM, "Mở trang này trên trình duyệt:");
                y = self.para(g, m, y + 6.0 * s, w - 2.0 * m, 18.0 * s, ACCENT, &url);
                y = self.para(g, m, y + 20.0 * s, w - 2.0 * m, 15.0 * s, DIM, "rồi nhập mã:");
                let bx = Rect::new(m, y + 8.0 * s, w - 2.0 * m, 80.0 * s);
                g.rect(bx.x, bx.y, bx.w, bx.h, CARD);
                text_c(g, bx, 40.0 * s, TEXT, &code);
                self.para(g, m, bx.y + bx.h + 24.0 * s, w - 2.0 * m, 14.0 * s, DIM, "Đang chờ bạn xác nhận trên trình duyệt...");
            }
            Status::Failed(msg) => {
                self.para(g, m, y, w - 2.0 * m, 15.0 * s, WARN, &msg);
                retry = true;
            }
            Status::Working(msg) => {
                self.para(g, m, y, w - 2.0 * m, 15.0 * s, DIM, &msg);
            }
            _ => {
                self.para(g, m, y, w - 2.0 * m, 15.0 * s, DIM, "Đang kết nối...");
            }
        }
        if retry {
            let b = Rect::new(m, h - 36.0 * s - 76.0 * s, w - 2.0 * m, 52.0 * s);
            vgrad(g, b, ACCENT, ACCENT_DK);
            text_c(g, b, 16.0 * s, 0x022b25ff, "THỬ LẠI");
            if hit(b, tap) {
                self.ms_started = false;
                *self.auth.lock().unwrap() = Status::Idle;
            }
        }
    }

    fn page_ely(&mut self, g: &mut Gfx, tap: Option<(f32, f32)>) {
        let (back, top) = self.header(g, "Đăng nhập Ely.by", tap);
        if back {
            *self.auth.lock().unwrap() = Status::Idle;
            self.fields = Default::default();
            self.go(Page::Accounts);
            return;
        }
        let st = self.auth.lock().unwrap().clone();
        if let Status::NeedTotp = st {
            self.need_totp = true;
            self.focus = 2;
            *self.auth.lock().unwrap() = Status::Idle;
        }
        let s = g.scale;
        let (w, h) = (g.w as f32, g.h as f32);
        let m = 16.0 * s;
        let n = if self.need_totp { 3 } else { 2 };
        let labels = ["Tên hoặc email", "Mật khẩu", "Mã xác thực 2 lớp"];
        let mut y = top + 16.0 * s;
        for i in 0..n {
            let bx = Rect::new(m, y, w - 2.0 * m, 56.0 * s);
            g.rect(bx.x, bx.y, bx.w, bx.h, CARD);
            g.rect(bx.x, bx.y + bx.h - 2.0 * s, bx.w, 2.0 * s, if self.focus == i { ACCENT } else { LINE });
            if self.fields[i].is_empty() {
                text_v(g, bx.x + 16.0 * s, bx, 16.0 * s, DIM, labels[i]);
            } else {
                let shown = if i == 1 { "*".repeat(self.fields[i].chars().count()) } else { self.fields[i].clone() };
                text_v(g, bx.x + 16.0 * s, bx, 18.0 * s, TEXT, &shown);
            }
            if hit(bx, tap) {
                self.focus = i;
            }
            y += 64.0 * s;
        }
        let kh = 300.0 * s;
        match &st {
            Status::Working(msg) => {
                self.para(g, m, y + 4.0 * s, w - 2.0 * m, 13.0 * s, DIM, msg);
            }
            Status::Failed(msg) => {
                self.para(g, m, y + 4.0 * s, w - 2.0 * m, 13.0 * s, WARN, msg);
            }
            _ => {
                if self.need_totp && self.fields[2].is_empty() {
                    self.para(g, m, y + 4.0 * s, w - 2.0 * m, 13.0 * s, WARN, "Tài khoản bật xác thực 2 lớp: nhập mã rồi bấm XONG");
                }
            }
        }

        let area = Rect::new(0.0, h - 36.0 * s - kh, w, kh);
        let working = matches!(st, Status::Working(_));
        match self.kb.draw(g, area, tap) {
            Some(Kb::Ch(c)) => {
                if self.fields[self.focus].chars().count() < 64 {
                    self.fields[self.focus].push(c);
                }
            }
            Some(Kb::Back) => {
                self.fields[self.focus].pop();
            }
            Some(Kb::Cancel) => {
                *self.auth.lock().unwrap() = Status::Idle;
                self.fields = Default::default();
                self.go(Page::Accounts);
            }
            Some(Kb::Done) => {
                if self.focus + 1 < n {
                    self.focus += 1;
                } else if !working && !self.fields[0].is_empty() && !self.fields[1].is_empty() {
                    let totp = if self.need_totp { self.fields[2].clone() } else { String::new() };
                    auth::ely_login(self.auth.clone(), self.fields[0].clone(), self.fields[1].clone(), totp);
                }
            }
            _ => {}
        }
    }
}
