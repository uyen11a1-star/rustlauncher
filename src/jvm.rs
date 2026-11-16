//! Khởi động JVM ngay trong tiến trình hiện tại qua libjli (JLI_Launch), cách PojavLauncher/Amethyst làm.
//! Chỉ dùng FFI libc thuần, không cần crate ngoài. Mỗi tiến trình chỉ chạy được MỘT lần JVM.
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
    fn setenv(k: *const c_char, v: *const c_char, overwrite: c_int) -> c_int;
    fn chdir(p: *const c_char) -> c_int;
    fn pipe(fds: *mut c_int) -> c_int;
    fn dup(fd: c_int) -> c_int;
    fn dup2(a: c_int, b: c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    fn atexit(cb: extern "C" fn()) -> c_int;
}

const RTLD_LAZY: c_int = 1;
const RTLD_GLOBAL: c_int = if cfg!(any(target_os = "linux", target_os = "android")) { 0x100 } else { 0x8 };

type JliLaunch = unsafe extern "C" fn(
    argc: c_int,
    argv: *const *const c_char,
    jargc: c_int,
    jargv: *const *const c_char,
    appclassc: c_int,
    appclassv: *const *const c_char,
    fullversion: *const c_char,
    dotversion: *const c_char,
    pname: *const c_char,
    lname: *const c_char,
    javaargs: u8,
    cpwildcard: u8,
    javaw: u8,
    ergo: i32,
) -> c_int;


static LOG_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Đặt file nhật ký bền (ghi và sync ngay) để đọc lại sau khi app bị văng.
pub fn set_crash_log(p: PathBuf) {
    let _ = LOG_PATH.set(p);
}

pub fn breadcrumb(msg: &str) {
    if let Some(p) = LOG_PATH.get() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{msg}");
            let _ = f.sync_all();
        }
    }
}

extern "C" fn on_exit() {
    breadcrumb("!! exit() được gọi trong tiến trình (JVM hoặc JLI thoát cả app)");
}

pub struct JvmConfig {
    /// Thư mục gốc JRE (chứa `release`, `lib`, `bin`).
    pub java_home: PathBuf,
    /// Thư mục chứa libjvm/libjava... (`lib/aarch64` hoặc `lib`).
    pub lib_dir: PathBuf,
    /// Thư mục làm việc và HOME của JVM.
    pub work_dir: PathBuf,
    /// Thư mục tạm.
    pub tmp_dir: PathBuf,
    /// Thư mục chứa các .so của app (để nạp bridge sau này), có thể rỗng.
    pub extra_lib_dirs: Vec<PathBuf>,
    /// Tham số JVM + lớp chính + tham số game (không gồm đường dẫn `java`).
    pub args: Vec<String>,
}

fn cs(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| format!("chuỗi chứa byte NUL: {s:?}"))
}

fn dl_err() -> String {
    unsafe {
        let e = dlerror();
        if e.is_null() {
            "không rõ lỗi".into()
        } else {
            CStr::from_ptr(e).to_string_lossy().into_owned()
        }
    }
}

fn dlopen_path(p: &Path) -> Result<*mut c_void, String> {
    let c = cs(&p.to_string_lossy())?;
    let h = unsafe { dlopen(c.as_ptr(), RTLD_LAZY | RTLD_GLOBAL) };
    if h.is_null() {
        Err(format!("dlopen {} thất bại: {}", p.display(), dl_err()))
    } else {
        Ok(h)
    }
}

fn set_env(k: &str, v: &str) -> Result<(), String> {
    let (k, v) = (cs(k)?, cs(v)?);
    if unsafe { setenv(k.as_ptr(), v.as_ptr(), 1) } != 0 {
        return Err("setenv thất bại".into());
    }
    Ok(())
}

/// Danh sách thư mục cho LD_LIBRARY_PATH của JVM.
pub fn ld_library_path(cfg: &JvmConfig) -> String {
    let l = &cfg.lib_dir;
    // libjvm (server/) PHẢI đứng đầu: JLI chỉ coi LD_LIBRARY_PATH là đủ khi mục đầu là thư mục libjvm,
    // nếu không nó tự execve lại chính mình (trên Android: "trying to exec .../bin/java: Permission denied").
    let mut v: Vec<String> = vec![
        l.join("server").display().to_string(),
        l.join("jli").display().to_string(),
        l.join("client").display().to_string(),
        l.display().to_string(),
    ];
    for d in &cfg.extra_lib_dirs {
        v.push(d.display().to_string());
    }
    v.join(":")
}

/// Tìm libjli.so: Android JRE đặt trong `jli/`, JDK desktop đặt ngay trong lib.
pub fn find_libjli(lib_dir: &Path) -> Option<PathBuf> {
    [lib_dir.join("jli/libjli.so"), lib_dir.join("libjli.so")]
        .into_iter()
        .find(|p| p.exists())
}

/// Chạy JVM trong tiến trình này, chặn tới khi JVM kết thúc. `on_line` nhận từng dòng stdout/stderr.
pub fn run(cfg: &JvmConfig, on_line: Arc<dyn Fn(String) + Send + Sync>) -> Result<i32, String> {
    let libjli = find_libjli(&cfg.lib_dir)
        .ok_or_else(|| format!("không thấy libjli.so trong {}", cfg.lib_dir.display()))?;

    std::fs::create_dir_all(&cfg.work_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cfg.tmp_dir).map_err(|e| e.to_string())?;

    unsafe { atexit(on_exit) };
    breadcrumb(&format!("run: libjli={}", libjli.display()));
    let ld = ld_library_path(cfg);
    // Nếu đã có đủ đường dẫn của mình (vd. JLI desktop tự exec lại chính nó) thì giữ nguyên.
    let have = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
    let ld_full = if have.contains(&ld) { have } else if have.is_empty() { ld.clone() } else { format!("{ld}:{have}") };
    set_env("LD_LIBRARY_PATH", &ld_full)?;
    set_env("JAVA_HOME", &cfg.java_home.to_string_lossy())?;
    set_env("HOME", &cfg.work_dir.to_string_lossy())?;
    set_env("TMPDIR", &cfg.tmp_dir.to_string_lossy())?;

    // Android: linker của app không đọc lại LD_LIBRARY_PATH, phải báo qua hàm ẩn của libdl.
    #[cfg(target_os = "android")]
    unsafe {
        let libdl = cs("libdl.so")?;
        let h = dlopen(libdl.as_ptr(), RTLD_LAZY);
        if !h.is_null() {
            let name = cs("android_update_LD_LIBRARY_PATH")?;
            let f = dlsym(h, name.as_ptr());
            if !f.is_null() {
                let f: unsafe extern "C" fn(*const c_char) = std::mem::transmute(f);
                let c = cs(&ld_full)?;
                f(c.as_ptr());
            }
        }
    }

    let wd = cs(&cfg.work_dir.to_string_lossy())?;
    if unsafe { chdir(wd.as_ptr()) } != 0 {
        return Err("chdir thất bại".into());
    }

    // Nạp trước các lib lõi theo thứ tự phụ thuộc (lỗi từng cái chỉ ghi lại, JVM sẽ tự báo nếu thiếu thật).
    let mut notes = Vec::new();
    let h_jli = dlopen_path(&libjli)?;
    for name in ["server/libjvm.so", "libverify.so", "libjava.so", "libnet.so", "libnio.so"] {
        let p = cfg.lib_dir.join(name);
        if p.exists() {
            if let Err(e) = dlopen_path(&p) {
                notes.push(e);
            }
        }
    }
    for n in notes {
        on_line(format!("[cảnh báo] {n}"));
    }

    breadcrumb("đã nạp libjli và các lib lõi");
    let sym = cs("JLI_Launch")?;
    let f = unsafe { dlsym(h_jli, sym.as_ptr()) };
    if f.is_null() {
        return Err(format!("không thấy JLI_Launch: {}", dl_err()));
    }
    let jli_launch: JliLaunch = unsafe { std::mem::transmute(f) };

    // argv
    let java_bin = cfg.java_home.join("bin/java").display().to_string();
    let mut all: Vec<String> = vec![java_bin.clone()];
    all.push(format!("-Djava.home={}", cfg.java_home.display()));
    all.push(format!("-Djava.io.tmpdir={}", cfg.tmp_dir.display()));
    all.push(format!("-Duser.home={}", cfg.work_dir.display()));
    all.extend(cfg.args.iter().cloned());
    let cstrs: Vec<CString> = all.iter().map(|s| cs(s)).collect::<Result<_, _>>()?;
    let mut argv: Vec<*const c_char> = cstrs.iter().map(|c| c.as_ptr()).collect();
    argv.push(std::ptr::null());

    // Bắt stdout/stderr: pipe -> luồng đọc.
    let mut fds = [0 as c_int; 2];
    if unsafe { pipe(fds.as_mut_ptr()) } != 0 {
        return Err("pipe thất bại".into());
    }
    let (rfd, wfd) = (fds[0], fds[1]);
    let (saved_out, saved_err) = unsafe { (dup(1), dup(2)) };
    unsafe {
        dup2(wfd, 1);
        dup2(wfd, 2);
        close(wfd);
    }
    let cb = on_line.clone();
    let reader = std::thread::spawn(move || {
        let mut pending: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe { read(rfd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            pending.extend_from_slice(&buf[..n as usize]);
            while let Some(i) = pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = pending.drain(..=i).collect();
                cb(String::from_utf8_lossy(&line[..line.len() - 1]).trim_end_matches('\r').to_string());
            }
        }
        if !pending.is_empty() {
            cb(String::from_utf8_lossy(&pending).into_owned());
        }
        unsafe { close(rfd) };
    });

    // KHÔNG reset signal: HotSpot tự lưu handler cũ của ART rồi chuyển tiếp các lỗi không phải của nó.
    // Reset về SIG_DFL làm mất handler SIGSEGV/SIGPIPE của ART và app bị văng ngay sau khi JVM xong.
    breadcrumb("sắp gọi JLI_Launch");

    let full = cs("1.8.0-internal")?;
    let dot = cs("1.8")?;
    let pname = cs(&java_bin)?;
    let code = unsafe {
        jli_launch(
            (argv.len() - 1) as c_int,
            argv.as_ptr(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            full.as_ptr(),
            dot.as_ptr(),
            pname.as_ptr(),
            pname.as_ptr(),
            0,
            1,
            0,
            0,
        )
    };

    breadcrumb(&format!("JLI_Launch trả về {code}"));
    // Trả stdout/stderr, đóng đầu ghi để luồng đọc gặp EOF.
    unsafe {
        dup2(saved_out, 1);
        dup2(saved_err, 2);
        close(saved_out);
        close(saved_err);
    }
    let _ = reader.join();
    Ok(code)
}

/// Đọc `release` của JRE: (JAVA_VERSION, OS_ARCH).
pub fn read_release(java_home: &Path) -> Option<(String, String)> {
    let mut s = String::new();
    std::fs::File::open(java_home.join("release")).ok()?.read_to_string(&mut s).ok()?;
    let get = |key: &str| -> Option<String> {
        let i = s.find(key)? + key.len();
        let rest = &s[i..];
        Some(rest[..rest.find('"')?].to_string())
    };
    Some((get("JAVA_VERSION=\"")?, get("OS_ARCH=\"")?))
}

/// Chọn thư mục lib thực sự chứa libjvm: `lib/<arch>` nếu có, không thì `lib`.
pub fn pick_lib_dir(java_home: &Path, arch: &str) -> PathBuf {
    let a = java_home.join("lib").join(arch);
    if a.exists() {
        a
    } else {
        java_home.join("lib")
    }
}


/// Sau khi giải nén: thư mục gốc JRE là thư mục chứa `release` (ngay tại `dir` hoặc trong một thư mục con duy nhất).
pub fn find_java_home(dir: &Path) -> Option<PathBuf> {
    if dir.join("release").exists() {
        return Some(dir.to_path_buf());
    }
    let subs: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subs.into_iter().find(|p| p.join("release").exists())
}

/// JRE Android đóng gói `libfreetype.so.6`; JVM tìm `libfreetype.so` nên đổi tên (như Amethyst).
pub fn fix_freetype(lib_dir: &Path) -> std::io::Result<()> {
    let from = lib_dir.join("libfreetype.so.6");
    let to = lib_dir.join("libfreetype.so");
    if from.exists() && !to.exists() {
        std::fs::rename(&from, &to)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ld_path_has_all_dirs() {
        let c = JvmConfig {
            java_home: "/j".into(),
            lib_dir: "/j/lib/aarch64".into(),
            work_dir: "/w".into(),
            tmp_dir: "/t".into(),
            extra_lib_dirs: vec!["/n".into()],
            args: vec![],
        };
        let p = ld_library_path(&c);
        assert!(p.starts_with("/j/lib/aarch64/server:/j/lib/aarch64/jli"));
        assert!(p.ends_with(":/n"));
    }

    #[test]
    fn release_parse() {
        let d = std::env::temp_dir().join(format!("rel_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("release"), "IMPLEMENTOR=\"x\"\nJAVA_VERSION=\"21.0.5\"\nOS_ARCH=\"aarch64\"\n").unwrap();
        assert_eq!(read_release(&d), Some(("21.0.5".into(), "aarch64".into())));
        assert_eq!(read_release(Path::new("/không/có")), None);
    }

    #[test]
    fn home_and_freetype() {
        let d = std::env::temp_dir().join(format!("home_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("jdk/lib")).unwrap();
        assert_eq!(find_java_home(&d), None);
        std::fs::write(d.join("jdk/release"), "x").unwrap();
        assert_eq!(find_java_home(&d), Some(d.join("jdk")));
        std::fs::write(d.join("release"), "x").unwrap();
        assert_eq!(find_java_home(&d), Some(d.clone()));
        std::fs::write(d.join("jdk/lib/libfreetype.so.6"), "f").unwrap();
        fix_freetype(&d.join("jdk/lib")).unwrap();
        assert!(d.join("jdk/lib/libfreetype.so").exists());
        assert!(!d.join("jdk/lib/libfreetype.so.6").exists());
        fix_freetype(&d.join("jdk/lib")).unwrap();
    }

    #[test]
    fn lib_dir_pick() {
        let d = std::env::temp_dir().join(format!("pick_{}", std::process::id()));
        std::fs::create_dir_all(d.join("lib/aarch64")).unwrap();
        assert_eq!(pick_lib_dir(&d, "aarch64"), d.join("lib/aarch64"));
        assert_eq!(pick_lib_dir(&d, "x86_64"), d.join("lib"));
    }
}
