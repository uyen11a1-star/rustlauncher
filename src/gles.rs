//! FFI tối thiểu tới EGL và GLES 3 (tự viết, không crate ngoài). Dùng cho giao diện launcher.
#![allow(non_snake_case, non_upper_case_globals, dead_code)]
use std::ffi::{c_char, c_void};
use std::ptr::null_mut;

pub type EGLDisplay = *mut c_void;
pub type EGLConfig = *mut c_void;
pub type EGLContext = *mut c_void;
pub type EGLSurface = *mut c_void;

#[link(name = "EGL")]
extern "C" {
    pub fn eglGetDisplay(d: *mut c_void) -> EGLDisplay;
    pub fn eglInitialize(d: EGLDisplay, major: *mut i32, minor: *mut i32) -> u32;
    pub fn eglBindAPI(api: u32) -> u32;
    pub fn eglChooseConfig(d: EGLDisplay, attribs: *const i32, configs: *mut EGLConfig, size: i32, num: *mut i32) -> u32;
    pub fn eglGetConfigAttrib(d: EGLDisplay, c: EGLConfig, attr: i32, value: *mut i32) -> u32;
    pub fn eglCreateContext(d: EGLDisplay, c: EGLConfig, share: EGLContext, attribs: *const i32) -> EGLContext;
    pub fn eglCreateWindowSurface(d: EGLDisplay, c: EGLConfig, win: *mut c_void, attribs: *const i32) -> EGLSurface;
    pub fn eglMakeCurrent(d: EGLDisplay, draw: EGLSurface, read: EGLSurface, ctx: EGLContext) -> u32;
    pub fn eglSwapBuffers(d: EGLDisplay, s: EGLSurface) -> u32;
    pub fn eglDestroySurface(d: EGLDisplay, s: EGLSurface) -> u32;
    pub fn eglDestroyContext(d: EGLDisplay, c: EGLContext) -> u32;
    pub fn eglQuerySurface(d: EGLDisplay, s: EGLSurface, attr: i32, value: *mut i32) -> u32;
    pub fn eglGetError() -> i32;
    pub fn eglGetProcAddress(name: *const c_char) -> *mut c_void;
}

#[link(name = "android")]
extern "C" {
    pub fn ANativeWindow_setBuffersGeometry(w: *mut c_void, width: i32, height: i32, format: i32) -> i32;
}

#[link(name = "GLESv3")]
extern "C" {
    pub fn glCreateShader(t: u32) -> u32;
    pub fn glShaderSource(s: u32, count: i32, src: *const *const c_char, len: *const i32);
    pub fn glCompileShader(s: u32);
    pub fn glGetShaderiv(s: u32, p: u32, out: *mut i32);
    pub fn glGetShaderInfoLog(s: u32, max: i32, len: *mut i32, log: *mut c_char);
    pub fn glCreateProgram() -> u32;
    pub fn glAttachShader(p: u32, s: u32);
    pub fn glBindAttribLocation(p: u32, i: u32, name: *const c_char);
    pub fn glLinkProgram(p: u32);
    pub fn glGetProgramiv(p: u32, pname: u32, out: *mut i32);
    pub fn glGetProgramInfoLog(p: u32, max: i32, len: *mut i32, log: *mut c_char);
    pub fn glUseProgram(p: u32);
    pub fn glGetUniformLocation(p: u32, name: *const c_char) -> i32;
    pub fn glUniform2f(loc: i32, a: f32, b: f32);
    pub fn glUniform1i(loc: i32, v: i32);
    pub fn glGenBuffers(n: i32, out: *mut u32);
    pub fn glBindBuffer(t: u32, b: u32);
    pub fn glBufferData(t: u32, size: isize, data: *const c_void, usage: u32);
    pub fn glEnableVertexAttribArray(i: u32);
    pub fn glVertexAttribPointer(i: u32, size: i32, t: u32, norm: u8, stride: i32, ptr: *const c_void);
    pub fn glGenTextures(n: i32, out: *mut u32);
    pub fn glBindTexture(t: u32, tex: u32);
    pub fn glTexParameteri(t: u32, p: u32, v: i32);
    pub fn glTexImage2D(t: u32, level: i32, internal: i32, w: i32, h: i32, border: i32, fmt: u32, ty: u32, px: *const c_void);
    pub fn glTexSubImage2D(t: u32, level: i32, x: i32, y: i32, w: i32, h: i32, fmt: u32, ty: u32, px: *const c_void);
    pub fn glPixelStorei(p: u32, v: i32);
    pub fn glActiveTexture(t: u32);
    pub fn glViewport(x: i32, y: i32, w: i32, h: i32);
    pub fn glScissor(x: i32, y: i32, w: i32, h: i32);
    pub fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    pub fn glClear(mask: u32);
    pub fn glEnable(c: u32);
    pub fn glDisable(c: u32);
    pub fn glBlendFunc(s: u32, d: u32);
    pub fn glDrawArrays(mode: u32, first: i32, count: i32);
    pub fn glDrawElements(mode: u32, count: i32, ty: u32, offset: *const c_void);
    pub fn glGetError() -> u32;
    pub fn glUniformMatrix4fv(loc: i32, count: i32, transpose: u8, v: *const f32);
    pub fn glDepthFunc(f: u32);
    pub fn glDeleteBuffers(n: i32, b: *const u32);
}

pub const GL_TEXTURE_2D: u32 = 0x0DE1;
pub const GL_ARRAY_BUFFER: u32 = 0x8892;
pub const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
pub const GL_UNSIGNED_INT: u32 = 0x1405;
pub const GL_STREAM_DRAW: u32 = 0x88E0;
pub const GL_FLOAT: u32 = 0x1406;
pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
pub const GL_TRIANGLES: u32 = 4;
pub const GL_BLEND: u32 = 0x0BE2;
pub const GL_SCISSOR_TEST: u32 = 0x0C11;
pub const GL_SRC_ALPHA: u32 = 0x0302;
pub const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
pub const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
pub const GL_VERTEX_SHADER: u32 = 0x8B31;
pub const GL_FRAGMENT_SHADER: u32 = 0x8B30;
pub const GL_COMPILE_STATUS: u32 = 0x8B81;
pub const GL_LINK_STATUS: u32 = 0x8B82;
pub const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
pub const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
pub const GL_TEXTURE_WRAP_S: u32 = 0x2802;
pub const GL_TEXTURE_WRAP_T: u32 = 0x2803;
pub const GL_LINEAR: i32 = 0x2601;
pub const GL_CLAMP_TO_EDGE: i32 = 0x812F;
pub const GL_R8: i32 = 0x8229;
pub const GL_RED: u32 = 0x1903;
pub const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;
pub const GL_TEXTURE0: u32 = 0x84C0;
pub const GL_DEPTH_TEST: u32 = 0x0B71;
pub const GL_CULL_FACE: u32 = 0x0B44;
pub const GL_LEQUAL: u32 = 0x0203;
pub const GL_DEPTH_BUFFER_BIT: u32 = 0x0100;
pub const GL_RGBA: u32 = 0x1908;
pub const GL_RGBA8: i32 = 0x8058;
pub const GL_NEAREST: i32 = 0x2600;
pub const GL_STATIC_DRAW: u32 = 0x88E4;

const EGL_NONE: i32 = 0x3038;
const EGL_DEPTH_SIZE: i32 = 0x3025;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_WINDOW_BIT: i32 = 4;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_NATIVE_VISUAL_ID: i32 = 0x302E;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;

/// EGL: display + context sống suốt app, surface đi theo cửa sổ Android.
pub struct Egl {
    pub display: EGLDisplay,
    pub config: EGLConfig,
    pub context: EGLContext,
    pub surface: EGLSurface,
}

impl Egl {
    pub fn new() -> Result<Egl, String> {
        unsafe {
            let display = eglGetDisplay(null_mut());
            if display.is_null() {
                return Err("eglGetDisplay thất bại".into());
            }
            let (mut ma, mut mi) = (0, 0);
            if eglInitialize(display, &mut ma, &mut mi) == 0 {
                return Err(format!("eglInitialize lỗi {:#x}", eglGetError()));
            }
            eglBindAPI(EGL_OPENGL_ES_API);
            let mut config: EGLConfig = null_mut();
            let mut n = 0;
            for (renderable, depth) in [(0x40, 24), (0x40, 16), (4, 16)] {
                // 0x40 = ES3, 4 = ES2
                let attribs = [
                    EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
                    EGL_RENDERABLE_TYPE, renderable,
                    EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
                    EGL_DEPTH_SIZE, depth,
                    EGL_NONE,
                ];
                if eglChooseConfig(display, attribs.as_ptr(), &mut config, 1, &mut n) != 0 && n > 0 {
                    break;
                }
                n = 0;
            }
            if n == 0 {
                return Err("không có EGL config phù hợp".into());
            }
            Ok(Egl { display, config, context: null_mut(), surface: null_mut() })
        }
    }

    /// Gắn vào cửa sổ Android, tạo context nếu chưa có, đặt làm hiện hành. Trả về (rộng, cao) pixel.
    pub fn attach(&mut self, window: *mut c_void) -> Result<(i32, i32), String> {
        unsafe {
            let mut fmt = 0;
            eglGetConfigAttrib(self.display, self.config, EGL_NATIVE_VISUAL_ID, &mut fmt);
            ANativeWindow_setBuffersGeometry(window, 0, 0, fmt);
            let s = eglCreateWindowSurface(self.display, self.config, window, [EGL_NONE].as_ptr());
            if s.is_null() {
                return Err(format!("eglCreateWindowSurface lỗi {:#x}", eglGetError()));
            }
            self.surface = s;
            if self.context.is_null() {
                let attribs = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
                self.context = eglCreateContext(self.display, self.config, null_mut(), attribs.as_ptr());
                if self.context.is_null() {
                    return Err(format!("eglCreateContext lỗi {:#x}", eglGetError()));
                }
            }
            if eglMakeCurrent(self.display, s, s, self.context) == 0 {
                return Err(format!("eglMakeCurrent lỗi {:#x}", eglGetError()));
            }
            Ok(self.size())
        }
    }

    pub fn size(&self) -> (i32, i32) {
        let (mut w, mut h) = (0, 0);
        unsafe {
            eglQuerySurface(self.display, self.surface, EGL_WIDTH, &mut w);
            eglQuerySurface(self.display, self.surface, EGL_HEIGHT, &mut h);
        }
        (w, h)
    }

    /// Nhả surface (Android sắp huỷ cửa sổ, hoặc nhường cửa sổ cho game).
    pub fn detach(&mut self) {
        unsafe {
            eglMakeCurrent(self.display, null_mut(), null_mut(), null_mut());
            if !self.surface.is_null() {
                eglDestroySurface(self.display, self.surface);
                self.surface = null_mut();
            }
        }
    }

    pub fn swap(&self) -> bool {
        !self.surface.is_null() && unsafe { eglSwapBuffers(self.display, self.surface) != 0 }
    }
}
