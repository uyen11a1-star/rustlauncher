//! Danh sách tài khoản lưu trong file `accounts.txt` (mỗi dòng `offline<TAB>tên`, dòng `sel<TAB>số` là tài khoản đang chọn).
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Offline,
    Microsoft,
    ElyBy,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Offline => "offline",
            Kind::Microsoft => "microsoft",
            Kind::ElyBy => "elyby",
        }
    }

    fn from_tag(s: &str) -> Option<Kind> {
        match s {
            "offline" => Some(Kind::Offline),
            "microsoft" => Some(Kind::Microsoft),
            "elyby" => Some(Kind::ElyBy),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Offline => "Ngoại tuyến",
            Kind::Microsoft => "Microsoft",
            Kind::ElyBy => "Ely.by",
        }
    }
}

/// `token`/`refresh`/`extra` dùng cho tài khoản trực tuyến (Ely.by: extra = clientToken; Microsoft: extra = Client ID).
#[derive(Clone)]
pub struct Account {
    pub kind: Kind,
    pub name: String,
    pub uuid: String,
    pub token: String,
    pub refresh: String,
    pub extra: String,
}

impl Account {
    pub fn offline(name: &str) -> Account {
        Account { kind: Kind::Offline, name: name.to_string(), uuid: String::new(), token: String::new(), refresh: String::new(), extra: String::new() }
    }
}

pub struct Accounts {
    pub list: Vec<Account>,
    pub sel: usize,
    path: PathBuf,
}

/// Tên người chơi: 1-16 ký tự chữ, số hoặc dấu gạch dưới.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 16 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Accounts {
    pub fn load(root: &Path) -> Accounts {
        let path = root.join("accounts.txt");
        let mut list = Vec::new();
        let mut sel = 0usize;
        if let Ok(t) = std::fs::read_to_string(&path) {
            for line in t.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                let get = |i: usize| f.get(i).copied().unwrap_or("").to_string();
                if f[0] == "sel" {
                    sel = get(1).trim().parse().unwrap_or(0);
                } else if let Some(kind) = Kind::from_tag(f[0]) {
                    let name = get(1);
                    if (kind == Kind::Offline && valid_name(&name)) || (kind != Kind::Offline && !name.is_empty()) {
                        list.push(Account { kind, name, uuid: get(2), token: get(3), refresh: get(4), extra: get(5) });
                    }
                }
            }
        }
        if sel >= list.len() {
            sel = 0;
        }
        Accounts { list, sel, path }
    }

    fn save(&self) {
        let mut s = String::new();
        for a in &self.list {
            s.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\n", a.kind.tag(), a.name, a.uuid, a.token, a.refresh, a.extra));
        }
        s.push_str(&format!("sel\t{}\n", self.sel));
        let _ = std::fs::write(&self.path, s);
    }

    pub fn current(&self) -> Option<&Account> {
        self.list.get(self.sel)
    }

    pub fn add_offline(&mut self, name: &str) -> Result<(), &'static str> {
        if !valid_name(name) {
            return Err("Tên phải dài 1-16 ký tự, chỉ gồm chữ, số, _");
        }
        if self.list.iter().any(|a| a.kind == Kind::Offline && a.name.eq_ignore_ascii_case(name)) {
            return Err("Tên này đã có trong danh sách");
        }
        self.add(Account::offline(name));
        Ok(())
    }

    /// Thêm tài khoản (trùng loại và tên thì thay thế) và chọn nó.
    pub fn add(&mut self, a: Account) {
        match self.list.iter().position(|x| x.kind == a.kind && x.name.eq_ignore_ascii_case(&a.name)) {
            Some(i) => {
                self.list[i] = a;
                self.sel = i;
            }
            None => {
                self.list.push(a);
                self.sel = self.list.len() - 1;
            }
        }
        self.save();
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.list.len() {
            self.list.remove(i);
            if self.sel >= self.list.len() && self.sel > 0 {
                self.sel -= 1;
            }
            self.save();
        }
    }

    pub fn select(&mut self, i: usize) {
        if i < self.list.len() {
            self.sel = i;
            self.save();
        }
    }
}
