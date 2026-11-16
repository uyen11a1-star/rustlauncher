//! Đọc và giải nén tar (ustar, GNU longname, pax) tự viết. Chống đường dẫn thoát khỏi thư mục đích.
use std::{
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    Hardlink,
    Other,
}

#[derive(Debug, Clone)]
pub struct Header {
    pub name: String,
    pub link: String,
    pub size: u64,
    pub mode: u32,
    pub kind: Kind,
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn octal(b: &[u8]) -> u64 {
    // Số lớn có thể mã hoá base-256 (bit cao = 1).
    if !b.is_empty() && b[0] & 0x80 != 0 {
        let mut v = (b[0] & 0x7f) as u64;
        for &c in &b[1..] {
            v = (v << 8) | c as u64;
        }
        return v;
    }
    let mut v = 0u64;
    for &c in b {
        if (b'0'..=b'7').contains(&c) {
            v = v * 8 + (c - b'0') as u64;
        } else if v > 0 || c == 0 {
            if c == 0 || c == b' ' {
                break;
            }
        }
    }
    v
}

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = r.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

fn skip<R: Read>(r: &mut R, mut n: u64) -> io::Result<()> {
    let mut buf = [0u8; 8192];
    while n > 0 {
        let want = n.min(buf.len() as u64) as usize;
        let k = r.read(&mut buf[..want])?;
        if k == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "tar bị cụt"));
        }
        n -= k as u64;
    }
    Ok(())
}

fn read_exact_vec<R: Read>(r: &mut R, n: u64) -> io::Result<Vec<u8>> {
    if n > 1 << 20 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "header phụ quá lớn"));
    }
    let mut v = vec![0u8; n as usize];
    r.read_exact(&mut v)?;
    Ok(v)
}

fn pad(size: u64) -> u64 {
    (512 - size % 512) % 512
}

/// Đọc header kế tiếp (đã gộp longname/pax). None khi hết archive.
pub fn next_header<R: Read>(r: &mut R) -> io::Result<Option<Header>> {
    let mut long_name: Option<String> = None;
    let mut long_link: Option<String> = None;
    let mut pax_size: Option<u64> = None;
    loop {
        let mut blk = [0u8; 512];
        let n = read_full(r, &mut blk)?;
        if n == 0 {
            return Ok(None);
        }
        if n < 512 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "header tar bị cụt"));
        }
        if blk.iter().all(|&b| b == 0) {
            return Ok(None);
        }
        // kiểm tra checksum
        let sum: u64 = blk
            .iter()
            .enumerate()
            .map(|(i, &b)| if (148..156).contains(&i) { 32 } else { b as u64 })
            .sum();
        if sum != octal(&blk[148..156]) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "checksum tar sai"));
        }
        let mut size = octal(&blk[124..136]);
        let typ = blk[156];
        match typ {
            b'L' | b'K' => {
                let data = read_exact_vec(r, size)?;
                skip(r, pad(size))?;
                let s = cstr(&data);
                if typ == b'L' {
                    long_name = Some(s);
                } else {
                    long_link = Some(s);
                }
                continue;
            }
            b'x' | b'g' => {
                let data = read_exact_vec(r, size)?;
                skip(r, pad(size))?;
                if typ == b'x' {
                    for (k, v) in parse_pax(&data) {
                        match k.as_str() {
                            "path" => long_name = Some(v),
                            "linkpath" => long_link = Some(v),
                            "size" => pax_size = v.parse().ok(),
                            _ => {}
                        }
                    }
                }
                continue;
            }
            _ => {}
        }
        if let Some(s) = pax_size {
            size = s;
        }
        let mut name = cstr(&blk[0..100]);
        if &blk[257..262] == b"ustar" {
            let prefix = cstr(&blk[345..500]);
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        }
        let kind = match typ {
            0 | b'0' | b'7' => Kind::File,
            b'5' => Kind::Dir,
            b'2' => Kind::Symlink,
            b'1' => Kind::Hardlink,
            _ => Kind::Other,
        };
        let kind = if kind == Kind::File && name.ends_with('/') { Kind::Dir } else { kind };
        return Ok(Some(Header {
            name: long_name.unwrap_or(name),
            link: long_link.unwrap_or_else(|| cstr(&blk[157..257])),
            size,
            mode: octal(&blk[100..108]) as u32,
            kind,
        }));
    }
}

fn parse_pax(mut d: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    while !d.is_empty() {
        let sp = match d.iter().position(|&c| c == b' ') {
            Some(p) => p,
            None => break,
        };
        let len: usize = match std::str::from_utf8(&d[..sp]).ok().and_then(|s| s.parse().ok()) {
            Some(l) if l > sp + 1 && l <= d.len() => l,
            _ => break,
        };
        let rec = &d[sp + 1..len];
        let rec = rec.strip_suffix(b"\n").unwrap_or(rec);
        if let Some(eq) = rec.iter().position(|&c| c == b'=') {
            out.push((
                String::from_utf8_lossy(&rec[..eq]).into_owned(),
                String::from_utf8_lossy(&rec[eq + 1..]).into_owned(),
            ));
        }
        d = &d[len..];
    }
    out
}

/// Ghép đường dẫn an toàn: bỏ `/` đầu, từ chối `..`.
pub fn safe_join(base: &Path, name: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for c in Path::new(name).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir | Component::RootDir => {}
            _ => return None,
        }
    }
    Some(out)
}

/// Giải nén cả tar vào `dest`. `progress(tên, tổng số file đã xử lý)`.
pub fn extract<R: Read>(
    mut r: R,
    dest: &Path,
    mut progress: impl FnMut(&str, usize),
) -> io::Result<usize> {
    fs::create_dir_all(dest)?;
    let mut count = 0usize;
    let mut hardlinks: Vec<(PathBuf, PathBuf)> = Vec::new();
    while let Some(h) = next_header(&mut r)? {
        let path = match safe_join(dest, &h.name) {
            Some(p) => p,
            None => {
                skip(&mut r, h.size + pad(h.size))?;
                continue;
            }
        };
        if path == dest {
            skip(&mut r, h.size + pad(h.size))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        match h.kind {
            Kind::Dir => {
                fs::create_dir_all(&path)?;
            }
            Kind::File => {
                let _ = fs::remove_file(&path);
                let mut f = fs::File::create(&path)?;
                let mut left = h.size;
                let mut buf = vec![0u8; 64 * 1024];
                while left > 0 {
                    let want = left.min(buf.len() as u64) as usize;
                    let k = r.read(&mut buf[..want])?;
                    if k == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "tar bị cụt"));
                    }
                    f.write_all(&buf[..k])?;
                    left -= k as u64;
                }
                drop(f);
                skip(&mut r, pad(h.size))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = if h.mode & 0o111 != 0 { 0o755 } else { 0o644 };
                    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(mode));
                }
            }
            Kind::Symlink => {
                skip(&mut r, h.size + pad(h.size))?;
                #[cfg(unix)]
                {
                    let _ = fs::remove_file(&path);
                    // Đích tương đối so với thư mục chứa link; chặn link trỏ ra ngoài thư mục đích.
                    let target = Path::new(&h.link);
                    let resolved = if target.is_absolute() {
                        None
                    } else {
                        let parent = path.parent().unwrap_or(dest);
                        normalize(&parent.join(target))
                    };
                    match resolved {
                        Some(p) if p.starts_with(dest) => {
                            std::os::unix::fs::symlink(&h.link, &path)?;
                        }
                        _ => {}
                    }
                }
            }
            Kind::Hardlink => {
                skip(&mut r, h.size + pad(h.size))?;
                if let Some(src) = safe_join(dest, &h.link) {
                    hardlinks.push((src, path.clone()));
                }
            }
            Kind::Other => {
                skip(&mut r, h.size + pad(h.size))?;
            }
        }
        count += 1;
        if count % 64 == 0 {
            progress(&h.name, count);
        }
    }
    for (src, dst) in hardlinks {
        let _ = fs::remove_file(&dst);
        if fs::hard_link(&src, &dst).is_err() {
            let _ = fs::copy(&src, &dst);
        }
    }
    Ok(count)
}

fn normalize(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("tarx_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn make_tar(src: &Path, out: &Path, fmt: &str) {
        let st = Command::new("tar")
            .args(["--format", fmt, "-cf"])
            .arg(out)
            .arg("-C")
            .arg(src)
            .arg(".")
            .status()
            .unwrap();
        assert!(st.success());
    }

    fn build_tree(root: &Path) {
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/hello.txt"), b"xin chao").unwrap();
        fs::write(root.join("a/big.bin"), vec![7u8; 100_000]).unwrap();
        let long = vec!["d".repeat(30); 4].join("/");
        fs::create_dir_all(root.join(&long)).unwrap();
        fs::write(root.join(&long).join(format!("{}.txt", "f".repeat(90))), b"long").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("b/hello.txt", root.join("a/link")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join("a/b/hello.txt"), fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn check(fmt: &str) {
        let src = tmp(&format!("src_{fmt}"));
        build_tree(&src);
        let tarf = tmp(&format!("t_{fmt}")).join("x.tar");
        make_tar(&src, &tarf, fmt);
        let dst = tmp(&format!("dst_{fmt}"));
        let n = extract(fs::File::open(&tarf).unwrap(), &dst, |_, _| {}).unwrap();
        assert!(n >= 6, "n={n}");
        assert_eq!(fs::read(dst.join("a/b/hello.txt")).unwrap(), b"xin chao");
        assert_eq!(fs::read(dst.join("a/big.bin")).unwrap().len(), 100_000);
        let long = vec!["d".repeat(30); 4].join("/");
        assert_eq!(
            fs::read(dst.join(&long).join(format!("{}.txt", "f".repeat(90)))).unwrap(),
            b"long"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::read_link(dst.join("a/link")).unwrap(), PathBuf::from("b/hello.txt"));
            assert_eq!(fs::read(dst.join("a/link")).unwrap(), b"xin chao");
            let m = fs::metadata(dst.join("a/b/hello.txt")).unwrap().permissions().mode();
            assert_eq!(m & 0o111, 0o111);
        }
    }

    #[test]
    fn gnu_format() {
        check("gnu");
    }
    #[test]
    fn ustar_format() {
        check("ustar");
    }
    #[test]
    fn pax_format() {
        check("pax");
    }

    #[test]
    fn rejects_traversal() {
        let base = Path::new("/x/y");
        assert!(safe_join(base, "../evil").is_none());
        assert!(safe_join(base, "a/../../evil").is_none());
        assert_eq!(safe_join(base, "/abs/p").unwrap(), PathBuf::from("/x/y/abs/p"));
        assert_eq!(safe_join(base, "./a/b").unwrap(), PathBuf::from("/x/y/a/b"));
    }

    #[test]
    fn truncated_is_error() {
        let src = tmp("src_trunc");
        build_tree(&src);
        let tarf = tmp("t_trunc").join("x.tar");
        make_tar(&src, &tarf, "gnu");
        let mut data = fs::read(&tarf).unwrap();
        data.truncate(data.len() / 2);
        let dst = tmp("dst_trunc");
        assert!(extract(&data[..], &dst, |_, _| {}).is_err());
    }
}
