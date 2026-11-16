//! Ma trận 4x4 theo cột (kiểu OpenGL), tự viết.
pub type Mat4 = [f32; 16];

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [0.0f32; 16];
    for c in 0..4 {
        for row in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[k * 4 + row] * b[c * 4 + k];
            }
            r[c * 4 + row] = s;
        }
    }
    r
}

pub fn translate(x: f32, y: f32, z: f32) -> Mat4 {
    [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0]
}

pub fn rot_x(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    [1.0, 0.0, 0.0, 0.0, 0.0, c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0]
}

pub fn rot_y(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    [c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0]
}

pub fn perspective(fovy: f32, aspect: f32, n: f32, f: f32) -> Mat4 {
    let t = 1.0 / (fovy / 2.0).tan();
    [t / aspect, 0.0, 0.0, 0.0, 0.0, t, 0.0, 0.0, 0.0, 0.0, (f + n) / (n - f), -1.0, 0.0, 0.0, 2.0 * f * n / (n - f), 0.0]
}

/// View của camera ở `eye`, quay `yaw` quanh Y rồi `pitch` quanh X (mặc định nhìn về -Z).
pub fn view(eye: [f32; 3], yaw: f32, pitch: f32) -> Mat4 {
    mul(&rot_x(-pitch), &mul(&rot_y(-yaw), &translate(-eye[0], -eye[1], -eye[2])))
}
