//! Đọc (bỏ qua) ô vật phẩm của giao thức 1.21.9: id + số lượng; phần "component" được bỏ qua bằng bộ thông dịch
//! chạy theo lược đồ trích từ minecraft-data (slotschema.json).
use std::{collections::HashMap, sync::OnceLock};

use crate::{
    chat,
    json::{self, Value},
    mcgui::net::rd_varint,
};

struct Schema {
    types: Value,
    comp: Value,
    ids: Vec<String>,
}

fn schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        let v = json::parse(include_str!("slotschema.json")).unwrap_or(Value::Null);
        let ids = v.get("ids").and_then(|a| a.as_arr()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        Schema { types: v.get("types").cloned().unwrap_or(Value::Null), comp: v.get("comp").cloned().unwrap_or(Value::Null), ids }
    })
}

#[derive(Clone)]
enum Val {
    I(i64),
    S(String),
    B(bool),
    N,
}

impl Val {
    fn key(&self) -> String {
        match self {
            Val::I(n) => n.to_string(),
            Val::S(s) => s.clone(),
            Val::B(b) => b.to_string(),
            Val::N => String::new(),
        }
    }
}

struct R<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> R<'a> {
    fn skip(&mut self, n: usize) -> Option<()> {
        if self.i.checked_add(n)? > self.b.len() {
            return None;
        }
        self.i += n;
        Some(())
    }
    fn varint(&mut self) -> Option<i32> {
        rd_varint(self.b, &mut self.i)
    }
}

type Scope = Vec<HashMap<String, Val>>;

fn lookup(sc: &Scope, path: &str) -> Val {
    let parts: Vec<&str> = path.split('/').collect();
    let ups = parts.iter().take_while(|p| **p == "..").count();
    let name = parts.last().copied().unwrap_or("");
    if sc.len() <= ups {
        return Val::N;
    }
    sc[sc.len() - 1 - ups].get(name).cloned().unwrap_or(Val::N)
}

fn read_ty(r: &mut R, ty: &Value, sc: &mut Scope, depth: u32) -> Option<Val> {
    if depth > 40 {
        return None;
    }
    match ty {
        Value::Str(name) => {
            let fixed = |r: &mut R, n: usize| r.skip(n).map(|_| Val::N);
            match name.as_str() {
                "varint" | "varlong" => r.varint().map(|v| Val::I(v as i64)),
                "i8" | "u8" => {
                    let v = *r.b.get(r.i)? as i64;
                    r.skip(1)?;
                    Some(Val::I(v))
                }
                "i16" | "u16" => fixed(r, 2),
                "i32" | "u32" | "f32" => fixed(r, 4),
                "i64" | "u64" | "f64" => fixed(r, 8),
                "bool" => {
                    let v = *r.b.get(r.i)?;
                    r.skip(1)?;
                    Some(Val::B(v != 0))
                }
                "void" => Some(Val::N),
                "UUID" => fixed(r, 16),
                "anonymousNbt" | "anonOptionalNbt" | "nbt" => {
                    chat::read_nbt(r.b, &mut r.i)?;
                    Some(Val::N)
                }
                "string" => {
                    let n = r.varint()?.max(0) as usize;
                    let s = String::from_utf8_lossy(r.b.get(r.i..r.i.checked_add(n)?)?).into_owned();
                    r.skip(n)?;
                    Some(Val::S(s))
                }
                "Slot" => {
                    read_slot_r(r)?;
                    Some(Val::N)
                }
                other => {
                    let t = schema().types.get(other)?;
                    read_ty(r, t, sc, depth + 1)
                }
            }
        }
        Value::Arr(a) if a.len() == 2 => {
            let ctor = a[0].as_str()?;
            let args = &a[1];
            match ctor {
                "container" => {
                    sc.push(HashMap::new());
                    let res = (|| {
                        container_fields(r, args, sc, depth)?;
                        Some(())
                    })();
                    sc.pop();
                    res.map(|_| Val::N)
                }
                "array" => {
                    let n = if let Some(ct) = args.get("countType") {
                        match read_ty(r, ct, sc, depth + 1)? {
                            Val::I(n) => n,
                            _ => 0,
                        }
                    } else {
                        match args.get("count") {
                            Some(Value::Num(n)) => *n as i64,
                            Some(Value::Str(s)) => match lookup(sc, s) {
                                Val::I(n) => n,
                                _ => 0,
                            },
                            _ => 0,
                        }
                    };
                    let et = args.get("type")?;
                    for _ in 0..n.max(0) {
                        read_ty(r, et, sc, depth + 1)?;
                    }
                    Some(Val::N)
                }
                "option" => {
                    let has = *r.b.get(r.i)?;
                    r.skip(1)?;
                    if has != 0 {
                        read_ty(r, args, sc, depth + 1)?;
                    }
                    Some(Val::N)
                }
                "switch" => {
                    let key = lookup(sc, args.get("compareTo")?.as_str()?).key();
                    let fields = args.get("fields")?;
                    match fields.get(&key).or_else(|| args.get("default")) {
                        Some(t) => read_ty(r, t, sc, depth + 1),
                        None => Some(Val::N),
                    }
                }
                "mapper" => {
                    let v = read_ty(r, args.get("type")?, sc, depth + 1)?;
                    let k = v.key();
                    Some(match args.get("mappings").and_then(|m| m.get(&k)).and_then(|x| x.as_str()) {
                        Some(s) => Val::S(s.to_string()),
                        None => v,
                    })
                }
                "bitfield" => {
                    let bits: f64 = args.as_arr()?.iter().filter_map(|f| f.get("size").and_then(|s| s.as_f64())).sum();
                    r.skip((bits / 8.0) as usize)?;
                    Some(Val::N)
                }
                "pstring" => {
                    let n = r.varint()?.max(0) as usize;
                    r.skip(n)?;
                    Some(Val::N)
                }
                "registryEntryHolder" => {
                    let v = r.varint()?;
                    if v == 0 {
                        read_ty(r, args.get("otherwise")?.get("type")?, sc, depth + 1)?;
                    }
                    Some(Val::N)
                }
                "registryEntryHolderSet" => {
                    let v = r.varint()?;
                    if v == 0 {
                        read_ty(r, args.get("base")?.get("type")?, sc, depth + 1)?;
                    } else {
                        let ot = args.get("otherwise")?.get("type")?;
                        for _ in 0..v - 1 {
                            read_ty(r, ot, sc, depth + 1)?;
                        }
                    }
                    Some(Val::N)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn container_fields(r: &mut R, fields: &Value, sc: &mut Scope, depth: u32) -> Option<()> {
    for f in fields.as_arr()? {
        let ty = f.get("type")?;
        let anon = f.get("anon").and_then(|a| a.as_bool()).unwrap_or(false);
        if anon {
            if let Value::Arr(a) = ty {
                if a.len() == 2 && a[0].as_str() == Some("container") {
                    container_fields(r, &a[1], sc, depth + 1)?;
                    continue;
                }
            }
        }
        let v = read_ty(r, ty, sc, depth + 1)?;
        if let Some(n) = f.get("name").and_then(|n| n.as_str()) {
            if let Some(top) = sc.last_mut() {
                top.insert(n.to_string(), v);
            }
        }
    }
    Some(())
}

fn read_slot_r(r: &mut R) -> Option<Option<(u32, u8)>> {
    let count = r.varint()?;
    if count <= 0 {
        return Some(None);
    }
    let item = r.varint()?;
    let added = r.varint()?;
    let removed = r.varint()?;
    let s = schema();
    for _ in 0..added.max(0) {
        let t = r.varint()?.max(0) as usize;
        let name = s.ids.get(t)?;
        let ty = s.comp.get(name)?;
        let mut sc: Scope = vec![HashMap::new()];
        read_ty(r, ty, &mut sc, 0)?;
    }
    for _ in 0..removed.max(0) {
        r.varint()?;
    }
    Some(Some((item as u32, count.min(255) as u8)))
}

/// Đọc một Slot tại `i`: None = lỗi/lệch dữ liệu; Some(None) = ô trống; Some(Some((id mạng, số lượng))).
pub fn read_slot(b: &[u8], i: &mut usize) -> Option<Option<(u32, u8)>> {
    let mut r = R { b, i: *i };
    let v = read_slot_r(&mut r)?;
    *i = r.i;
    Some(v)
}
