//! Tin nhắn: đọc NBT mạng của text component, đổi ra chuỗi kiểu cũ (§) để vẽ bằng font Minecraft.
use std::{collections::HashMap, sync::OnceLock};

pub enum Tag {
    S(String),
    C(Vec<(String, Tag)>),
    L(Vec<Tag>),
    N(i64),
    O,
}

/// Bảng dịch en_us (nạp từ client.jar khi vào game).
pub static LANG: OnceLock<HashMap<String, String>> = OnceLock::new();

fn take<'a>(b: &'a [u8], i: &mut usize, n: usize) -> Option<&'a [u8]> {
    let s = b.get(*i..i.checked_add(n)?)?;
    *i += n;
    Some(s)
}

fn be(b: &[u8], i: &mut usize, n: usize) -> Option<i64> {
    let s = take(b, i, n)?;
    let mut v: i64 = if s[0] & 0x80 != 0 { -1 } else { 0 };
    for x in s {
        v = (v << 8) | *x as i64;
    }
    Some(v)
}

fn str_(b: &[u8], i: &mut usize) -> Option<String> {
    let n = be(b, i, 2)? as u16 as usize;
    Some(String::from_utf8_lossy(take(b, i, n)?).into_owned())
}

fn payload(b: &[u8], i: &mut usize, t: u8, d: usize) -> Option<Tag> {
    if d > 32 {
        return None;
    }
    Some(match t {
        1 => Tag::N(be(b, i, 1)?),
        2 => Tag::N(be(b, i, 2)?),
        3 => Tag::N(be(b, i, 4)?),
        4 => Tag::N(be(b, i, 8)?),
        5 => {
            take(b, i, 4)?;
            Tag::O
        }
        6 => {
            take(b, i, 8)?;
            Tag::O
        }
        7 => {
            let n = be(b, i, 4)?.max(0) as usize;
            take(b, i, n)?;
            Tag::O
        }
        8 => Tag::S(str_(b, i)?),
        9 => {
            let et = be(b, i, 1)? as u8;
            let n = be(b, i, 4)?.max(0) as usize;
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(payload(b, i, et, d + 1)?);
            }
            Tag::L(v)
        }
        10 => {
            let mut v = Vec::new();
            loop {
                let tt = be(b, i, 1)? as u8;
                if tt == 0 {
                    break;
                }
                let k = str_(b, i)?;
                v.push((k, payload(b, i, tt, d + 1)?));
            }
            Tag::C(v)
        }
        11 => {
            let n = be(b, i, 4)?.max(0) as usize;
            take(b, i, n * 4)?;
            Tag::O
        }
        12 => {
            let n = be(b, i, 4)?.max(0) as usize;
            take(b, i, n * 8)?;
            Tag::O
        }
        _ => return None,
    })
}

/// NBT mạng không tên gốc.
pub fn read_nbt(b: &[u8], i: &mut usize) -> Option<Tag> {
    let t = be(b, i, 1)? as u8;
    if t == 0 {
        return Some(Tag::O);
    }
    payload(b, i, t, 0)
}

fn get<'a>(c: &'a [(String, Tag)], k: &str) -> Option<&'a Tag> {
    c.iter().find(|x| x.0 == k).map(|x| &x.1)
}

const COLORS: [(&str, char, u32); 16] = [
    ("black", '0', 0x000000),
    ("dark_blue", '1', 0x0000aa),
    ("dark_green", '2', 0x00aa00),
    ("dark_aqua", '3', 0x00aaaa),
    ("dark_red", '4', 0xaa0000),
    ("dark_purple", '5', 0xaa00aa),
    ("gold", '6', 0xffaa00),
    ("gray", '7', 0xaaaaaa),
    ("dark_gray", '8', 0x555555),
    ("blue", '9', 0x5555ff),
    ("green", 'a', 0x55ff55),
    ("aqua", 'b', 0x55ffff),
    ("red", 'c', 0xff5555),
    ("light_purple", 'd', 0xff55ff),
    ("yellow", 'e', 0xffff55),
    ("white", 'f', 0xffffff),
];

fn color_code(name: &str) -> Option<char> {
    if let Some(h) = name.strip_prefix('#') {
        let v = u32::from_str_radix(h, 16).ok()?;
        let d = |a: u32, b: u32| {
            let f = |s: u32| ((a >> s & 255) as i32 - (b >> s & 255) as i32).pow(2);
            f(0) + f(8) + f(16)
        };
        return COLORS.iter().min_by_key(|c| d(v, c.2)).map(|c| c.1);
    }
    COLORS.iter().find(|c| c.0 == name).map(|c| c.1)
}

fn flag(c: &[(String, Tag)], k: &str) -> bool {
    matches!(get(c, k), Some(Tag::N(1)))
}

/// Công thức dịch: thay %s và %n$s.
fn format(fmt: &str, args: &[String]) -> String {
    let mut out = String::new();
    let cs: Vec<char> = fmt.chars().collect();
    let (mut i, mut next) = (0, 0);
    while i < cs.len() {
        if cs[i] == '%' && i + 1 < cs.len() {
            if cs[i + 1] == '%' {
                out.push('%');
                i += 2;
                continue;
            }
            let mut j = i + 1;
            let mut num = String::new();
            while j < cs.len() && cs[j].is_ascii_digit() {
                num.push(cs[j]);
                j += 1;
            }
            if j + 1 < cs.len() && cs[j] == '$' && cs[j + 1] == 's' && !num.is_empty() {
                if let Some(a) = args.get(num.parse::<usize>().unwrap_or(1).saturating_sub(1)) {
                    out.push_str(a);
                }
                i = j + 2;
                continue;
            }
            if cs[i + 1] == 's' || cs[i + 1] == 'd' {
                if let Some(a) = args.get(next) {
                    out.push_str(a);
                }
                next += 1;
                i += 2;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

fn tr(key: &str) -> Option<String> {
    if let Some(s) = LANG.get().and_then(|l| l.get(key)) {
        return Some(s.clone());
    }
    Some(
        match key {
            "chat.type.text" => "<%s> %s",
            "chat.type.emote" => "* %s %s",
            "chat.type.announcement" => "[%s] %s",
            "chat.type.admin" => "[%s: %s]",
            "multiplayer.player.joined" => "%s joined the game",
            "multiplayer.player.left" => "%s left the game",
            "commands.message.display.incoming" => "%s whispers to you: %s",
            "commands.message.display.outgoing" => "You whisper to %s: %s",
            _ => return None,
        }
        .to_string(),
    )
}

/// Đổi component sang chuỗi có mã §. `st` là kiểu thừa kế (vd "§a§l").
pub fn legacy(t: &Tag, st: &str, out: &mut String) {
    match t {
        Tag::S(s) => out.push_str(s),
        Tag::N(n) => out.push_str(&n.to_string()),
        Tag::L(v) => {
            for x in v {
                legacy(x, st, out);
            }
        }
        Tag::O => {}
        Tag::C(c) => {
            let mut mine = st.to_string();
            if let Some(Tag::S(col)) = get(c, "color") {
                if let Some(ch) = color_code(col) {
                    mine = format!("§{ch}");
                }
            }
            for (k, ch) in [("bold", 'l'), ("italic", 'o'), ("underlined", 'n'), ("strikethrough", 'm'), ("obfuscated", 'k')] {
                if flag(c, k) {
                    mine.push('§');
                    mine.push(ch);
                }
            }
            if mine != st {
                out.push_str("§r");
                out.push_str(&mine);
            }
            if let Some(Tag::S(s)) = get(c, "text") {
                out.push_str(s);
            } else if let Some(Tag::S(k)) = get(c, "translate") {
                let mut args = Vec::new();
                if let Some(Tag::L(w)) = get(c, "with") {
                    for a in w {
                        let mut s = String::new();
                        legacy(a, &mine, &mut s);
                        if !mine.is_empty() || s.contains('§') {
                            s.push_str("§r");
                            s.push_str(&mine);
                        }
                        args.push(s);
                    }
                }
                match tr(k) {
                    Some(f) => out.push_str(&format(&f, &args)),
                    None => {
                        out.push_str(k);
                        for a in &args {
                            out.push(' ');
                            out.push_str(a);
                        }
                    }
                }
            } else if let Some(Tag::S(k)) = get(c, "keybind") {
                out.push_str(k);
            }
            if let Some(e) = get(c, "extra") {
                legacy(e, &mine, out);
            }
            if mine != st {
                out.push_str("§r");
                out.push_str(st);
            }
        }
    }
}

pub fn to_legacy(t: &Tag) -> String {
    let mut s = String::new();
    legacy(t, "", &mut s);
    s
}

/// Đặt tin theo loại chat (chỉ số trong registry chat_type của vanilla).
pub fn typed(kind: usize, name: &str, msg: &str) -> String {
    match kind {
        1 => format!("[{name}] {msg}"),
        2 => format!("{name} whispers to you: {msg}"),
        3 => format!("You whisper to {name}: {msg}"),
        6 => format!("* {name} {msg}"),
        _ => format!("<{name}> {msg}"),
    }
}
