//! Trạng thái game: người chơi, điều khiển cảm ứng, ma trận camera. Chỉ dùng thư viện chuẩn.

use crate::world::height_at;

pub type Mat4 = [[f32; 4]; 4]; // cột trước, hàng sau (khớp mat4x4 của WGSL)

const EYE_HEIGHT: f32 = 1.62;
const MOVE_SPEED: f32 = 6.0;
const LOOK_SENS: f32 = 0.004; // rad trên mỗi pixel
const FOV_DEG: f32 = 70.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Started,
    Moved,
    Ended,
}

struct Stick {
    id: u64,
    ox: f32,
    oy: f32,
    cx: f32,
    cy: f32,
}

struct Look {
    id: u64,
    lx: f32,
    ly: f32,
}

pub struct Game {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    screen: (f32, f32),
    stick: Option<Stick>,
    look: Option<Look>,
}

impl Game {
    pub fn new() -> Self {
        let h = height_at(0, 0) as f32;
        Self {
            pos: [0.5, h + 1.0 + EYE_HEIGHT, 0.5],
            yaw: 0.0,
            pitch: -0.2,
            screen: (1.0, 1.0),
            stick: None,
            look: None,
        }
    }

    pub fn set_screen(&mut self, w: f32, h: f32) {
        self.screen = (w.max(1.0), h.max(1.0));
    }

    /// Nửa trái màn hình: cần điều khiển di chuyển. Nửa phải: kéo để xoay camera.
    pub fn touch(&mut self, id: u64, phase: Phase, x: f32, y: f32) {
        match phase {
            Phase::Started => {
                if x < self.screen.0 * 0.5 {
                    if self.stick.is_none() {
                        self.stick = Some(Stick { id, ox: x, oy: y, cx: x, cy: y });
                    }
                } else if self.look.is_none() {
                    self.look = Some(Look { id, lx: x, ly: y });
                }
            }
            Phase::Moved => {
                if let Some(s) = &mut self.stick {
                    if s.id == id {
                        s.cx = x;
                        s.cy = y;
                    }
                }
                if let Some(l) = &mut self.look {
                    if l.id == id {
                        self.yaw += (x - l.lx) * LOOK_SENS;
                        self.pitch = (self.pitch - (y - l.ly) * LOOK_SENS).clamp(-1.5, 1.5);
                        l.lx = x;
                        l.ly = y;
                    }
                }
            }
            Phase::Ended => {
                if self.stick.as_ref().map_or(false, |s| s.id == id) {
                    self.stick = None;
                }
                if self.look.as_ref().map_or(false, |l| l.id == id) {
                    self.look = None;
                }
            }
        }
    }

    pub fn update(&mut self, dt: f32) {
        let radius = self.screen.0.min(self.screen.1) * 0.2;
        let (mut strafe, mut forward) = (0.0f32, 0.0f32);
        if let Some(s) = &self.stick {
            let dx = (s.cx - s.ox) / radius;
            let dy = (s.cy - s.oy) / radius;
            let len = (dx * dx + dy * dy).sqrt();
            if len > 0.1 {
                let k = len.min(1.0) / len;
                strafe = dx * k;
                forward = -dy * k;
            }
        }

        let (sy, cy) = self.yaw.sin_cos();
        // Hướng nhìn ngang: (sin yaw, -cos yaw). Hướng phải: (cos yaw, sin yaw).
        self.pos[0] += (sy * forward + cy * strafe) * MOVE_SPEED * dt;
        self.pos[2] += (-cy * forward + sy * strafe) * MOVE_SPEED * dt;

        // Mốc 1: bám mặt đất, chưa có va chạm hay trọng lực.
        let ground = height_at(self.pos[0].floor() as i32, self.pos[2].floor() as i32) as f32
            + 1.0
            + EYE_HEIGHT;
        self.pos[1] += (ground - self.pos[1]) * (1.0 - (-10.0 * dt).exp());
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let f = [sy * cp, sp, -cy * cp];
        let proj = perspective(FOV_DEG.to_radians(), aspect, 0.1, 400.0);
        mat_mul(&proj, &look_to(self.pos, f))
    }
}

// ---------- Toán ma trận ----------

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

pub fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut o = [[0.0f32; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[k][r] * b[c][k];
            }
            o[c][r] = s;
        }
    }
    o
}

/// Phối cảnh tay phải, độ sâu 0..1 (đúng quy ước của wgpu).
pub fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fovy * 0.5).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ]
}

/// Ma trận nhìn tay phải từ `eye` theo hướng đơn vị `f`.
pub fn look_to(eye: [f32; 3], f: [f32; 3]) -> Mat4 {
    let s = normalize(cross(f, [0.0, 1.0, 0.0]));
    let u = cross(s, f);
    [
        [s[0], u[0], -f[0], 0.0],
        [s[1], u[1], -f[1], 0.0],
        [s[2], u[2], -f[2], 0.0],
        [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform(m: &Mat4, p: [f32; 3]) -> [f32; 4] {
        let mut o = [0.0; 4];
        for r in 0..4 {
            o[r] = m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r];
        }
        o
    }

    #[test]
    fn projection_conventions() {
        let mut g = Game::new();
        g.pos = [0.0, 0.0, 0.0];
        g.yaw = 0.0;
        g.pitch = 0.0;
        let vp = g.view_proj(1.0);

        // Điểm ngay trước mặt (nhìn về -Z): giữa màn hình, độ sâu trong (0,1).
        let c = transform(&vp, [0.0, 0.0, -5.0]);
        assert!(c[0].abs() < 1e-4 && c[1].abs() < 1e-4);
        assert!((c[3] - 5.0).abs() < 1e-4);
        let z = c[2] / c[3];
        assert!(z > 0.0 && z < 1.0, "z={z}");

        // +X nằm bên phải, +Y nằm phía trên.
        assert!(transform(&vp, [2.0, 0.0, -5.0])[0] > 0.0);
        assert!(transform(&vp, [0.0, 2.0, -5.0])[1] > 0.0);

        // Điểm sau lưng có w < 0.
        assert!(transform(&vp, [0.0, 0.0, 5.0])[3] < 0.0);

        // Gần đẩy xa hơn thì độ sâu tăng.
        let zn = transform(&vp, [0.0, 0.0, -1.0]);
        let zf = transform(&vp, [0.0, 0.0, -50.0]);
        assert!(zn[2] / zn[3] < zf[2] / zf[3]);
    }

    #[test]
    fn yaw_turns_right() {
        let mut g = Game::new();
        g.pos = [0.0, 0.0, 0.0];
        g.pitch = 0.0;
        g.yaw = std::f32::consts::FRAC_PI_2; // quay phải 90 độ: nhìn về +X
        let vp = g.view_proj(1.0);
        let c = transform(&vp, [5.0, 0.0, 0.0]);
        assert!(c[0].abs() < 1e-3 && c[3] > 0.0);
    }

    #[test]
    fn stick_moves_forward_and_look_drags() {
        let mut g = Game::new();
        g.set_screen(2000.0, 1000.0);
        let (x0, z0) = (g.pos[0], g.pos[2]);
        g.touch(1, Phase::Started, 300.0, 700.0);
        g.touch(1, Phase::Moved, 300.0, 500.0); // kéo lên = đi tới
        for _ in 0..60 {
            g.update(1.0 / 60.0);
        }
        assert!(g.pos[2] < z0 - 1.0, "đi tới phải giảm z");
        assert!((g.pos[0] - x0).abs() < 0.01);
        g.touch(1, Phase::Ended, 300.0, 500.0);
        let z1 = g.pos[2];
        g.update(0.1);
        assert!((g.pos[2] - z1).abs() < 1e-6, "thả tay thì đứng yên");

        g.touch(2, Phase::Started, 1500.0, 500.0);
        g.touch(2, Phase::Moved, 1600.0, 500.0);
        assert!(g.yaw > 0.0);
        g.touch(2, Phase::Moved, 1600.0, 400.0); // kéo lên = nhìn lên
        assert!(g.pitch > -0.2 + 0.1);
    }
}
