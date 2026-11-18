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

pub fn rot_z(a: f32) -> Mat4 {
    let (s, c) = a.sin_cos();
    [c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
}

pub fn scale(x: f32, y: f32, z: f32) -> Mat4 {
    [x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 0.0, 1.0]
}

/// Nhân nhiều ma trận từ trái sang phải (như PoseStack: phép sau áp trong hệ tọa độ của phép trước).
pub fn chain(ms: &[Mat4]) -> Mat4 {
    let mut r = ms[0];
    for m in &ms[1..] {
        r = mul(&r, m);
    }
    r
}

pub fn perspective(fovy: f32, aspect: f32, n: f32, f: f32) -> Mat4 {
    let t = 1.0 / (fovy / 2.0).tan();
    [t / aspect, 0.0, 0.0, 0.0, 0.0, t, 0.0, 0.0, 0.0, 0.0, (f + n) / (n - f), -1.0, 0.0, 0.0, 2.0 * f * n / (n - f), 0.0]
}

/// View của camera ở `eye`, quay `yaw` quanh Y rồi `pitch` quanh X (mặc định nhìn về -Z).
pub fn view(eye: [f32; 3], yaw: f32, pitch: f32) -> Mat4 {
    mul(&rot_x(-pitch), &mul(&rot_y(-yaw), &translate(-eye[0], -eye[1], -eye[2])))
}

/// 6 mặt phẳng frustum từ ma trận MVP (Gribb-Hartmann), mỗi mặt [a,b,c,d], điểm trong khi a*x+b*y+c*z+d >= 0.
pub fn frustum(m: &Mat4) -> [[f32; 4]; 6] {
    let r = |i: usize| [m[i], m[4 + i], m[8 + i], m[12 + i]];
    let (r0, r1, r2, r3) = (r(0), r(1), r(2), r(3));
    let mut p = [[0.0f32; 4]; 6];
    for k in 0..4 {
        p[0][k] = r3[k] + r0[k];
        p[1][k] = r3[k] - r0[k];
        p[2][k] = r3[k] + r1[k];
        p[3][k] = r3[k] - r1[k];
        p[4][k] = r3[k] + r2[k];
        p[5][k] = r3[k] - r2[k];
    }
    p
}

/// Hộp [minx,miny,minz,maxx,maxy,maxz] có nằm (một phần) trong frustum không.
pub fn aabb_visible(f: &[[f32; 4]; 6], b: &[f32; 6]) -> bool {
    for p in f {
        let x = if p[0] >= 0.0 { b[3] } else { b[0] };
        let y = if p[1] >= 0.0 { b[4] } else { b[1] };
        let z = if p[2] >= 0.0 { b[5] } else { b[2] };
        if p[0] * x + p[1] * y + p[2] * z + p[3] < 0.0 {
            return false;
        }
    }
    true
}
