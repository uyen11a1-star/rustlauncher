//! Kiểm tra / xin quyền "quản lý mọi tệp" (Android 11+) bằng JNI viết tay.
//! Chỉ dùng vài hàm JNI: FindClass, GetStaticMethodID, CallStatic*MethodA, NewObjectA, startActivity.
use std::ffi::{c_char, c_void, CString};

type Env = *mut c_void;

unsafe fn fnp<T: Copy>(env: Env, idx: usize) -> T {
    let table = *(env as *const *const *const c_void);
    std::mem::transmute_copy(&*table.add(idx))
}

struct Jni {
    env: Env,
}

impl Jni {
    unsafe fn clear(&self) -> bool {
        let check: extern "C" fn(Env) -> u8 = fnp(self.env, 228);
        if check(self.env) != 0 {
            let clear: extern "C" fn(Env) = fnp(self.env, 17);
            clear(self.env);
            true
        } else {
            false
        }
    }
    unsafe fn class(&self, name: &str) -> *mut c_void {
        let f: extern "C" fn(Env, *const c_char) -> *mut c_void = fnp(self.env, 6);
        let n = CString::new(name).unwrap();
        let c = f(self.env, n.as_ptr());
        self.clear();
        c
    }
    unsafe fn static_method(&self, cls: *mut c_void, name: &str, sig: &str) -> *mut c_void {
        let f: extern "C" fn(Env, *mut c_void, *const c_char, *const c_char) -> *mut c_void = fnp(self.env, 113);
        let (n, s) = (CString::new(name).unwrap(), CString::new(sig).unwrap());
        let m = f(self.env, cls, n.as_ptr(), s.as_ptr());
        self.clear();
        m
    }
    unsafe fn method(&self, cls: *mut c_void, name: &str, sig: &str) -> *mut c_void {
        let f: extern "C" fn(Env, *mut c_void, *const c_char, *const c_char) -> *mut c_void = fnp(self.env, 33);
        let (n, s) = (CString::new(name).unwrap(), CString::new(sig).unwrap());
        let m = f(self.env, cls, n.as_ptr(), s.as_ptr());
        self.clear();
        m
    }
    unsafe fn string(&self, s: &str) -> *mut c_void {
        let f: extern "C" fn(Env, *const c_char) -> *mut c_void = fnp(self.env, 167);
        let c = CString::new(s).unwrap();
        f(self.env, c.as_ptr())
    }
}

/// Lấy JNIEnv của luồng hiện tại (gắn vào JVM nếu chưa gắn).
unsafe fn env_of(vm: *mut c_void) -> Option<Env> {
    let table = *(vm as *const *const *const c_void);
    let get_env: extern "C" fn(*mut c_void, *mut Env, i32) -> i32 = std::mem::transmute_copy(&*table.add(6));
    let mut env: Env = std::ptr::null_mut();
    if get_env(vm, &mut env, 0x0001_0006) == 0 && !env.is_null() {
        return Some(env);
    }
    let attach: extern "C" fn(*mut c_void, *mut Env, *mut c_void) -> i32 = std::mem::transmute_copy(&*table.add(4));
    if attach(vm, &mut env, std::ptr::null_mut()) == 0 && !env.is_null() {
        Some(env)
    } else {
        None
    }
}

/// true nếu app đã được phép đọc mọi tệp (hoặc Android cũ hơn 11, không cần quyền này).
pub fn has_all_files(vm: *mut c_void) -> bool {
    unsafe {
        let Some(env) = env_of(vm) else { return true };
        let j = Jni { env };
        let cls = j.class("android/os/Environment");
        if cls.is_null() {
            return true;
        }
        let m = j.static_method(cls, "isExternalStorageManager", "()Z");
        if m.is_null() {
            return true; // Android < 11
        }
        let call: extern "C" fn(Env, *mut c_void, *mut c_void, *const u64) -> u8 = fnp(env, 119);
        let r = call(env, cls, m, std::ptr::null());
        if j.clear() {
            return true;
        }
        r != 0
    }
}

/// Mở trang "Cho phép quản lý mọi tệp" của chính app. Trả về Err kèm lý do nếu không mở được.
pub fn request_all_files(vm: *mut c_void, activity: *mut c_void, package: &str) -> Result<(), String> {
    unsafe {
        let env = env_of(vm).ok_or("không gắn được luồng vào JVM")?;
        let j = Jni { env };
        let uri_cls = j.class("android/net/Uri");
        let intent_cls = j.class("android/content/Intent");
        if uri_cls.is_null() || intent_cls.is_null() {
            return Err("không tìm thấy lớp Uri/Intent".into());
        }
        let parse = j.static_method(uri_cls, "parse", "(Ljava/lang/String;)Landroid/net/Uri;");
        let ctor = j.method(intent_cls, "<init>", "(Ljava/lang/String;Landroid/net/Uri;)V");
        if parse.is_null() || ctor.is_null() {
            return Err("không tìm thấy hàm Uri.parse / Intent()".into());
        }
        let uri_str = j.string(&format!("package:{package}"));
        let call_obj: extern "C" fn(Env, *mut c_void, *mut c_void, *const u64) -> *mut c_void = fnp(env, 116);
        let uri = call_obj(env, uri_cls, parse, [uri_str as u64].as_ptr());
        if j.clear() || uri.is_null() {
            return Err("Uri.parse lỗi".into());
        }
        let action = j.string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION");
        let new_obj: extern "C" fn(Env, *mut c_void, *mut c_void, *const u64) -> *mut c_void = fnp(env, 30);
        let intent = new_obj(env, intent_cls, ctor, [action as u64, uri as u64].as_ptr());
        if j.clear() || intent.is_null() {
            return Err("tạo Intent lỗi".into());
        }
        let get_class: extern "C" fn(Env, *mut c_void) -> *mut c_void = fnp(env, 31);
        let act_cls = get_class(env, activity);
        let start = j.method(act_cls, "startActivity", "(Landroid/content/Intent;)V");
        if start.is_null() {
            return Err("không thấy Activity.startActivity".into());
        }
        let call_void: extern "C" fn(Env, *mut c_void, *mut c_void, *const u64) = fnp(env, 63);
        call_void(env, activity, start, [intent as u64].as_ptr());
        if j.clear() {
            return Err("startActivity ném lỗi (máy không có trang này?)".into());
        }
        Ok(())
    }
}

/// Bật bàn phím hệ thống: InputMethodManager.toggleSoftInput(SHOW_FORCED, 0).
/// NativeActivity không có ô nhập nên showSoftInput thường không hiện gì; toggleSoftInput thì luôn hiện.
/// Phím gõ sau đó tới app dưới dạng KeyEvent.
pub fn show_keyboard(vm: *mut c_void, activity: *mut c_void) -> Result<(), String> {
    unsafe {
        let env = env_of(vm).ok_or("không gắn được luồng vào JVM")?;
        let j = Jni { env };
        let get_class: extern "C" fn(Env, *mut c_void) -> *mut c_void = fnp(env, 31);
        let act_cls = get_class(env, activity);
        let gss = j.method(act_cls, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;");
        if gss.is_null() {
            return Err("không thấy getSystemService".into());
        }
        let name = j.string("input_method");
        let call_obj: extern "C" fn(Env, *mut c_void, *mut c_void, *const u64) -> *mut c_void = fnp(env, 36);
        let imm = call_obj(env, activity, gss, [name as u64].as_ptr());
        if j.clear() || imm.is_null() {
            return Err("không lấy được InputMethodManager".into());
        }
        let imm_cls = get_class(env, imm);
        let toggle = j.method(imm_cls, "toggleSoftInput", "(II)V");
        if toggle.is_null() {
            return Err("không thấy toggleSoftInput".into());
        }
        let call_void: extern "C" fn(Env, *mut c_void, *mut c_void, *const u64) = fnp(env, 63);
        call_void(env, imm, toggle, [2u64, 0u64].as_ptr());
        if j.clear() {
            return Err("toggleSoftInput ném lỗi".into());
        }
        Ok(())
    }
}

/// Đổi hướng màn hình: ngang (SCREEN_ORIENTATION_SENSOR_LANDSCAPE = 6) hoặc dọc (SCREEN_ORIENTATION_PORTRAIT = 1).
pub fn set_orientation(vm: *mut c_void, activity: *mut c_void, landscape: bool) -> Result<(), String> {
    unsafe {
        let env = env_of(vm).ok_or("không gắn được luồng vào JVM")?;
        let j = Jni { env };
        let get_class: extern "C" fn(Env, *mut c_void) -> *mut c_void = fnp(env, 31);
        let cls = get_class(env, activity);
        let m = j.method(cls, "setRequestedOrientation", "(I)V");
        if m.is_null() {
            return Err("không thấy setRequestedOrientation".into());
        }
        let call_void: extern "C" fn(Env, *mut c_void, *mut c_void, *const u64) = fnp(env, 63);
        call_void(env, activity, m, [if landscape { 6u64 } else { 1u64 }].as_ptr());
        if j.clear() {
            return Err("setRequestedOrientation ném lỗi".into());
        }
        Ok(())
    }
}
