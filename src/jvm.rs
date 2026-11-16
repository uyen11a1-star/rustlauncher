//! Khởi động JVM ngay trong tiến trình hiện tại bằng JNI_CreateJavaVM (JNI tự viết, không qua JLI).
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
    breadcrumb("!! exit() được gọi trong tiến trình");
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
    /// Tuỳ chọn JVM (-Xmx..., -D..., -Djava.class.path=...).
    pub args: Vec<String>,
    /// Lớp chứa main (a.b.C). None = chỉ in thông tin phiên bản Java.
    pub main_class: Option<String>,
    /// Tham số truyền cho main.
    pub app_args: Vec<String>,
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

// ---------- JNI tối thiểu (tự viết): chỉ những hàm cần để tạo JVM và gọi main ----------

#[repr(C)]
struct JavaVMOption {
    option_string: *const c_char,
    extra_info: *mut c_void,
}

#[repr(C)]
struct JavaVMInitArgs {
    version: i32,
    n_options: i32,
    options: *mut JavaVMOption,
    ignore_unrecognized: u8,
}

type CreateJavaVm = unsafe extern "C" fn(*mut *mut c_void, *mut *mut c_void, *mut c_void) -> i32;
/// JNIEnv* và JavaVM* đều là con trỏ tới con trỏ tới bảng hàm.
type Table = *mut *const *const c_void;

const JNI_VERSION_1_8: i32 = 0x0001_0008;
const FIND_CLASS: usize = 6;
const EXCEPTION_DESCRIBE: usize = 16;
const EXCEPTION_CLEAR: usize = 17;
const GET_STATIC_METHOD_ID: usize = 113;
const CALL_STATIC_OBJECT_METHOD_A: usize = 116;
const CALL_STATIC_VOID_METHOD_A: usize = 143;
const NEW_STRING_UTF: usize = 167;
const GET_STRING_UTF_CHARS: usize = 169;
const NEW_OBJECT_ARRAY: usize = 172;
const SET_OBJECT_ARRAY_ELEMENT: usize = 174;
const EXCEPTION_CHECK: usize = 228;

unsafe fn slot(t: Table, idx: usize) -> *const c_void {
    *(*t).add(idx)
}

unsafe fn find_class(env: Table, name: &str) -> Result<*mut c_void, String> {
    let f: unsafe extern "C" fn(Table, *const c_char) -> *mut c_void = std::mem::transmute(slot(env, FIND_CLASS));
    let c = cs(name)?;
    let r = f(env, c.as_ptr());
    if r.is_null() {
        describe(env);
        return Err(format!("không tìm thấy lớp {name}"));
    }
    Ok(r)
}

unsafe fn static_method(env: Table, cls: *mut c_void, name: &str, sig: &str) -> Result<*mut c_void, String> {
    let f: unsafe extern "C" fn(Table, *mut c_void, *const c_char, *const c_char) -> *mut c_void =
        std::mem::transmute(slot(env, GET_STATIC_METHOD_ID));
    let (n, s) = (cs(name)?, cs(sig)?);
    let r = f(env, cls, n.as_ptr(), s.as_ptr());
    if r.is_null() {
        describe(env);
        return Err(format!("không thấy phương thức {name}{sig}"));
    }
    Ok(r)
}

unsafe fn describe(env: Table) {
    let chk: unsafe extern "C" fn(Table) -> u8 = std::mem::transmute(slot(env, EXCEPTION_CHECK));
    if chk(env) != 0 {
        let d: unsafe extern "C" fn(Table) = std::mem::transmute(slot(env, EXCEPTION_DESCRIBE));
        d(env);
        let c: unsafe extern "C" fn(Table) = std::mem::transmute(slot(env, EXCEPTION_CLEAR));
        c(env);
    }
}

unsafe fn new_string(env: Table, s: &str) -> Result<*mut c_void, String> {
    let f: unsafe extern "C" fn(Table, *const c_char) -> *mut c_void = std::mem::transmute(slot(env, NEW_STRING_UTF));
    let c = cs(s)?;
    let r = f(env, c.as_ptr());
    if r.is_null() {
        return Err("NewStringUTF thất bại".into());
    }
    Ok(r)
}

/// System.getProperty(key)
unsafe fn sys_prop(env: Table, key: &str) -> Result<String, String> {
    let cls = find_class(env, "java/lang/System")?;
    let m = static_method(env, cls, "getProperty", "(Ljava/lang/String;)Ljava/lang/String;")?;
    let k = new_string(env, key)?;
    let call: unsafe extern "C" fn(Table, *mut c_void, *mut c_void, *const u64) -> *mut c_void =
        std::mem::transmute(slot(env, CALL_STATIC_OBJECT_METHOD_A));
    let args = [k as usize as u64];
    let r = call(env, cls, m, args.as_ptr());
    if r.is_null() {
        describe(env);
        return Ok(String::new());
    }
    let get: unsafe extern "C" fn(Table, *mut c_void, *mut u8) -> *const c_char = std::mem::transmute(slot(env, GET_STRING_UTF_CHARS));
    let p = get(env, r, std::ptr::null_mut());
    Ok(if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() })
}

/// Gọi `main(String[])` của lớp `class` (dạng a.b.C). Trả về true nếu không có ngoại lệ.
unsafe fn call_main(env: Table, class: &str, args: &[String]) -> Result<bool, String> {
    let cls = find_class(env, &class.replace('.', "/"))?;
    let m = static_method(env, cls, "main", "([Ljava/lang/String;)V")?;
    let str_cls = find_class(env, "java/lang/String")?;
    let new_arr: unsafe extern "C" fn(Table, i32, *mut c_void, *mut c_void) -> *mut c_void =
        std::mem::transmute(slot(env, NEW_OBJECT_ARRAY));
    let arr = new_arr(env, args.len() as i32, str_cls, std::ptr::null_mut());
    if arr.is_null() {
        return Err("NewObjectArray thất bại".into());
    }
    let set: unsafe extern "C" fn(Table, *mut c_void, i32, *mut c_void) = std::mem::transmute(slot(env, SET_OBJECT_ARRAY_ELEMENT));
    for (i, a) in args.iter().enumerate() {
        set(env, arr, i as i32, new_string(env, a)?);
    }
    let call: unsafe extern "C" fn(Table, *mut c_void, *mut c_void, *const u64) = std::mem::transmute(slot(env, CALL_STATIC_VOID_METHOD_A));
    let jargs = [arr as usize as u64];
    call(env, cls, m, jargs.as_ptr());
    let chk: unsafe extern "C" fn(Table) -> u8 = std::mem::transmute(slot(env, EXCEPTION_CHECK));
    if chk(env) != 0 {
        describe(env);
        return Ok(false);
    }
    Ok(true)
}

/// Chạy JVM trong tiến trình này (tạo VM trực tiếp bằng JNI_CreateJavaVM, KHÔNG qua JLI nên không có
/// chuyện JLI tự exec lại hay gọi exit). Chặn tới khi xong. `on_line` nhận từng dòng stdout/stderr.
/// Nếu `cfg.main_class` là None thì in thông tin phiên bản Java.
pub fn run(cfg: &JvmConfig, on_line: Arc<dyn Fn(String) + Send + Sync>) -> Result<i32, String> {
    std::fs::create_dir_all(&cfg.work_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cfg.tmp_dir).map_err(|e| e.to_string())?;

    unsafe { atexit(on_exit) };
    let ld = ld_library_path(cfg);
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

    // Nạp trước các lib lõi (libjli nếu có: libjava có thể cần). Lỗi lẻ chỉ ghi lại.
    if let Some(jli) = find_libjli(&cfg.lib_dir) {
        if let Err(e) = dlopen_path(&jli) {
            on_line(format!("[cảnh báo] {e}"));
        }
    }
    let libjvm = cfg.lib_dir.join("server/libjvm.so");
    breadcrumb(&format!("nạp {}", libjvm.display()));
    let h_jvm = dlopen_path(&libjvm)?;
    for name in ["libverify.so", "libjava.so", "libnet.so", "libnio.so"] {
        let p = cfg.lib_dir.join(name);
        if p.exists() {
            if let Err(e) = dlopen_path(&p) {
                on_line(format!("[cảnh báo] {e}"));
            }
        }
    }
    let sym = cs("JNI_CreateJavaVM")?;
    let f = unsafe { dlsym(h_jvm, sym.as_ptr()) };
    if f.is_null() {
        return Err(format!("không thấy JNI_CreateJavaVM: {}", dl_err()));
    }
    let create: CreateJavaVm = unsafe { std::mem::transmute(f) };

    // Tuỳ chọn JVM
    let mut opts: Vec<String> = vec![
        format!("-Djava.home={}", cfg.java_home.display()),
        format!("-Djava.library.path={}", ld),
        format!("-Djava.io.tmpdir={}", cfg.tmp_dir.display()),
        format!("-Duser.home={}", cfg.work_dir.display()),
    ];
    opts.extend(cfg.args.iter().cloned());
    let copts: Vec<CString> = opts.iter().map(|s| cs(s)).collect::<Result<_, _>>()?;
    let mut jopts: Vec<JavaVMOption> = copts
        .iter()
        .map(|c| JavaVMOption { option_string: c.as_ptr(), extra_info: std::ptr::null_mut() })
        .collect();
    let mut init = JavaVMInitArgs {
        version: JNI_VERSION_1_8,
        n_options: jopts.len() as i32,
        options: jopts.as_mut_ptr(),
        ignore_unrecognized: 1,
    };

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
    breadcrumb("sắp gọi JNI_CreateJavaVM");
    let mut vm: *mut c_void = std::ptr::null_mut();
    let mut env: *mut c_void = std::ptr::null_mut();
    let rc = unsafe { create(&mut vm, &mut env, &mut init as *mut _ as *mut c_void) };
    breadcrumb(&format!("JNI_CreateJavaVM trả về {rc}"));

    let mut code = 0;
    let mut err: Option<String> = None;
    if rc != 0 {
        err = Some(format!("JNI_CreateJavaVM lỗi {rc}"));
    } else {
        let env = env as Table;
        let r: Result<(), String> = unsafe {
            match &cfg.main_class {
                None => (|| {
                    for (label, key) in [
                        ("java.version", "java.version"),
                        ("java.vm.name", "java.vm.name"),
                        ("java.vm.version", "java.vm.version"),
                        ("java.runtime.name", "java.runtime.name"),
                        ("os.arch", "os.arch"),
                    ] {
                        println!("{label} = {}", sys_prop(env, key)?);
                    }
                    Ok(())
                })(),
                Some(c) => call_main(env, c, &cfg.app_args).map(|ok| {
                    if !ok {
                        code = 1;
                    }
                }),
            }
        };
        if let Err(e) = r {
            err = Some(e);
            code = 1;
        }
        // Không gọi DestroyJavaVM: nó chờ mọi luồng non-daemon, có thể treo; JVM chỉ chạy một lần mỗi tiến trình.
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    breadcrumb(&format!("xong, mã {code}"));

    // Trả stdout/stderr. LƯU Ý: JVM còn sống vẫn có thể ghi ra fd 1/2 trước khi trả; sau khi trả là fd gốc.
    unsafe {
        dup2(saved_out, 1);
        dup2(saved_err, 2);
        close(saved_out);
        close(saved_err);
    }
    let _ = reader.join();
    match err {
        Some(e) => Err(e),
        None => Ok(code),
    }
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
            main_class: None,
            app_args: vec![],
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
