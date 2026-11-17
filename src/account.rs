//! Danh sách tài khoản lưu trong file `accounts.txt` (mỗi dòng `offline<TAB>tên`, dòng `sel<TAB>số` là tài khoản đang chọn).
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Offline,
}

#[derive(Clone)]
pub struct Account {
    pub kind: Kind,
    pub name: String,
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
                let mut it = line.splitn(2, '\t');
                match (it.next(), it.next()) {
                    (Some("offline"), Some(n)) if valid_name(n) => list.push(Account { kind: Kind::Offline, name: n.to_string() }),
                    (Some("sel"), Some(n)) => sel = n.trim().parse().unwrap_or(0),
                    _ => {}
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
            s.push_str(&format!("offline\t{}\n", a.name));
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
        if self.list.iter().any(|a| a.name.eq_ignore_ascii_case(name)) {
            return Err("Tên này đã có trong danh sách");
        }
        self.list.push(Account { kind: Kind::Offline, name: name.to_string() });
        self.sel = self.list.len() - 1;
        self.save();
        Ok(())
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
