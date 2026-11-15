//! Trạng thái game: người chơi, vật lý, cảm ứng, đập/đặt khối, ma trận camera.
//! Chỉ dùng thư viện chuẩn.

use crate::ui::Layout;
use crate::world::{height_at, World, AIR, CH, HOTBAR};

pub type Mat4 = [[f32; 4]; 4]; // cột trước, hàng sau (khớp mat4x4 của WGSL)

const EYE_HEIGHT: f32 = 1.62;
const HALF_W: f32 = 0.3; // nửa bề rộng thân người
const BODY_H: f32 = 1.8;
const SKIN: f32 = 0.001; // khe hở nhỏ giữa người và tường để không bị kẹt
const WALK_SPEED: f32 = 5.5;
const FLY_SPEED: f32 = 12.0;
const GRAVITY: f32 = 28.0;
const JUMP_V: f32 = 9.0; // nhảy cao khoảng 1,45 khối
const MAX_FALL: f32 = 40.0;
const REACH: f32 = 5.0;
const LONG_PRESS: f32 = 0.3; // giữ lâu hơn mức này thì đập khối
const LOOK_SENS: f32 = 0.004; // rad trên mỗi pixel
const FOV_DEG: f32 = 70.0;
const MAX_STEP: f32 = 0.2; // mỗi bước di chuyển tối đa, tránh xuyên tường khi rơi nhanh

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
    sx: f32,
    sy: f32,
    held: f32,   // thời gian đã giữ mà chưa kéo
    moved: bool, // đã kéo đủ xa: đây là thao tác xoay, không phải chạm
    broke: bool, // lần giữ này đã đập khối rồi
}

/// Kết quả bắn tia: `block` là khối bị trúng, `prev` là ô trống ngay trước nó (chỗ để đặt khối).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hit {
    pub block: [i32; 3],
    pub prev: [i32; 3],
}

pub struct Game {
    /// Vị trí chân người chơi.
    pub pos: [f32; 3],
    vel_y: f32,
    on_ground: bool,
    pub flying: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub selected: usize,
    layout: Layout,
    stick: Option<Stick>,
    look: Option<Look>,
    jump_touch: Option<u64>,
    want_place: bool,
    want_break: bool,
    target: Option<Hit>,
}

impl Game {
    pub fn new() -> Self {
        let h = height_at(0, 0) as f32;
        Self {
            pos: [0.5, h + 1.0 + SKIN, 0.5],
            vel_y: 0.0,
            on_ground: false,
            flying: false,
            yaw: 0.0,
            pitch: -0.2,
            selected: 0,
            layout: Layout::new(1.0, 1.0),
            stick: None,
            look: None,
            jump_touch: None,
            want_place: false,
            want_break: false,
            target: None,
        }
    }

    pub fn set_screen(&mut self, w: f32, h: f32) {
        self.layout = Layout::new(w.max(1.0), h.max(1.0));
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn eye(&self) -> [f32; 3] {
        [self.pos[0], self.pos[1] + EYE_HEIGHT, self.pos[2]]
    }

    pub fn look_dir(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [sy * cp, sp, -cy * cp]
    }

    /// Khối đang được nhắm tới (để vẽ khung viền).
    pub fn target(&self) -> Option<Hit> {
        self.target
    }

    pub fn jump_held(&self) -> bool {
        self.jump_touch.is_some()
    }

    /// (điểm chạm đầu, điểm chạm hiện tại) của cần điều khiển, nếu đang dùng.
    pub fn stick_visual(&self) -> Option<([f32; 2], [f32; 2])> {
        self.stick
            .as_ref()
            .map(|s| ([s.ox, s.oy], [s.cx, s.cy]))
    }

    fn unit(&self) -> f32 {
        self.layout.unit
    }

    /// Nút/thanh vật phẩm được ưu tiên. Nửa trái màn hình còn lại: cần điều khiển.
    /// Nửa phải: kéo để xoay; chạm nhanh để đặt khối; giữ để đập khối.
    pub fn touch(&mut self, id: u64, phase: Phase, x: f32, y: f32) {
        match phase {
            Phase::Started => {
                if self.layout.jump.contains(x, y) {
                    if self.jump_touch.is_none() {
                        self.jump_touch = Some(id);
                    }
                    return;
                }
                if self.layout.fly.contains(x, y) {
                    self.flying = !self.flying;
                    self.vel_y = 0.0;
                    return;
                }
                for (i, r) in self.layout.hotbar.iter().enumerate() {
                    if r.contains(x, y) {
                        self.selected = i;
                        return;
                    }
                }
                if x < self.layout.screen.0 * 0.5 {
                    if self.stick.is_none() {
                        self.stick = Some(Stick { id, ox: x, oy: y, cx: x, cy: y });
                    }
                } else if self.look.is_none() {
                    self.look = Some(Look {
                        id,
                        lx: x,
                        ly: y,
                        sx: x,
                        sy: y,
                        held: 0.0,
                        moved: false,
                        broke: false,
                    });
                }
            }
            Phase::Moved => {
                if let Some(s) = &mut self.stick {
                    if s.id == id {
                        s.cx = x;
                        s.cy = y;
                    }
                }
                let slop = 0.012 * self.unit();
                if let Some(l) = &mut self.look {
                    if l.id == id {
                        if !l.moved {
                            let (dx, dy) = (x - l.sx, y - l.sy);
                            if dx * dx + dy * dy > slop * slop {
                                l.moved = true;
                            }
                        }
                        if l.moved {
                            self.yaw += (x - l.lx) * LOOK_SENS;
                            self.pitch = (self.pitch - (y - l.ly) * LOOK_SENS).clamp(-1.5, 1.5);
                            l.lx = x;
                            l.ly = y;
                        }
                    }
                }
            }
            Phase::Ended => {
                if self.jump_touch == Some(id) {
                    self.jump_touch = None;
                }
                if self.stick.as_ref().map_or(false, |s| s.id == id) {
                    self.stick = None;
                }
                if let Some(l) = &self.look {
                    if l.id == id {
                        // Chạm nhanh, không kéo, không đập: đặt khối.
                        if !l.moved && !l.broke {
                            self.want_place = true;
                        }
                        self.look = None;
                    }
                }
            }
        }
    }

    /// Cập nhật một khung hình. Trả về các chunk bị sửa và cần dựng lại mesh.
    pub fn update(&mut self, dt: f32, world: &mut World) -> Vec<(i32, i32)> {
        // --- Cần điều khiển ---
        let radius = (self.unit() * 0.2).max(1.0);
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

        // --- Vận tốc ---
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let jump_held = self.jump_touch.is_some();
        // Hướng nhìn ngang: (sin yaw, -cos yaw). Hướng phải: (cos yaw, sin yaw).
        let (vx, vz);
        if self.flying {
            vx = (sy * cp * forward + cy * strafe) * FLY_SPEED;
            vz = (-cy * cp * forward + sy * strafe) * FLY_SPEED;
            // Bay: đi tới theo hướng nhìn (nhìn xuống thì hạ xuống), giữ nút nhảy để bay lên.
            self.vel_y = sp * forward * FLY_SPEED + if jump_held { FLY_SPEED * 0.7 } else { 0.0 };
        } else {
            vx = (sy * forward + cy * strafe) * WALK_SPEED;
            vz = (-cy * forward + sy * strafe) * WALK_SPEED;
            if jump_held && self.on_ground {
                self.vel_y = JUMP_V;
                self.on_ground = false;
            }
            self.vel_y = (self.vel_y - GRAVITY * dt).max(-MAX_FALL);
        }

        // --- Di chuyển có va chạm, từng trục một ---
        self.move_axis(world, 0, vx * dt);
        self.move_axis(world, 2, vz * dt);
        let dy = self.vel_y * dt;
        if self.move_axis(world, 1, dy) {
            if dy < 0.0 {
                self.on_ground = true;
            }
            self.vel_y = 0.0;
        } else {
            self.on_ground = false;
        }

        // --- Giữ lâu thì đập khối, lặp lại mỗi LONG_PRESS giây ---
        if let Some(l) = &mut self.look {
            if !l.moved {
                l.held += dt;
                if l.held >= LONG_PRESS {
                    l.held = 0.0;
                    l.broke = true;
                    self.want_break = true;
                }
            }
        }

        // --- Nhắm khối và thực hiện đập/đặt ---
        self.target = raycast(world, self.eye(), self.look_dir(), REACH);
        let mut dirty = Vec::new();

        if self.want_break {
            self.want_break = false;
            if let Some(h) = self.target {
                // Lớp đáy (y = 0) không đập được, kẻo lộ ra khoảng trống.
                if h.block[1] > 0 {
                    dirty.extend(world.set(h.block[0], h.block[1], h.block[2], AIR));
                }
            }
        }
        if self.want_place {
            self.want_place = false;
            if let Some(h) = self.target {
                let [px, py, pz] = h.prev;
                if (0..CH).contains(&py) && !self.body_overlaps(px, py, pz) {
                    dirty.extend(world.set(px, py, pz, HOTBAR[self.selected]));
                }
            }
        }
        if !dirty.is_empty() {
            self.target = raycast(world, self.eye(), self.look_dir(), REACH);
        }
        dirty
    }

    /// Thân người có chạm ô khối (x, y, z) không (để không đặt khối đè lên chính mình).
    fn body_overlaps(&self, x: i32, y: i32, z: i32) -> bool {
        let (x, y, z) = (x as f32, y as f32, z as f32);
        self.pos[0] + HALF_W > x
            && self.pos[0] - HALF_W < x + 1.0
            && self.pos[1] + BODY_H > y
            && self.pos[1] < y + 1.0
            && self.pos[2] + HALF_W > z
            && self.pos[2] - HALF_W < z + 1.0
    }

    /// Di chuyển theo một trục (0 = x, 1 = y, 2 = z). Trả về true nếu bị chặn.
    fn move_axis(&mut self, world: &World, axis: usize, delta: f32) -> bool {
        if delta == 0.0 {
            return false;
        }
        // Chia nhỏ để mỗi bước chỉ vượt qua tối đa một ranh giới khối.
        let n = ((delta.abs() / MAX_STEP).ceil() as i32).max(1);
        let d = delta / n as f32;
        for _ in 0..n {
            let mut p = self.pos;
            p[axis] += d;
            if !body_hits(world, p) {
                self.pos = p;
                continue;
            }
            // Bị chặn: áp sát mặt khối đã chạm, chừa một khe SKIN.
            self.pos[axis] = if d > 0.0 {
                let front = if axis == 1 { p[1] + BODY_H } else { p[axis] + HALF_W };
                let edge = front.floor();
                if axis == 1 {
                    edge - BODY_H - SKIN
                } else {
                    edge - HALF_W - SKIN
                }
            } else {
                let front = if axis == 1 { p[1] } else { p[axis] - HALF_W };
                let edge = front.floor() + 1.0;
                if axis == 1 {
                    edge + SKIN
                } else {
                    edge + HALF_W + SKIN
                }
            };
            return true;
        }
        false
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let proj = perspective(FOV_DEG.to_radians(), aspect, 0.1, 400.0);
        mat_mul(&proj, &look_to(self.eye(), self.look_dir()))
    }
}

/// Hộp bao của người chơi tại `p` (vị trí chân) có chạm khối đặc nào không.
fn body_hits(world: &World, p: [f32; 3]) -> bool {
    let x0 = (p[0] - HALF_W).floor() as i32;
    let x1 = (p[0] + HALF_W).floor() as i32;
    let y0 = p[1].floor() as i32;
    let y1 = (p[1] + BODY_H).floor() as i32;
    let z0 = (p[2] - HALF_W).floor() as i32;
    let z1 = (p[2] + HALF_W).floor() as i32;
    for y in y0..=y1 {
        for z in z0..=z1 {
            for x in x0..=x1 {
                if world.is_solid(x, y, z) {
                    return true;
                }
            }
        }
    }
    false
}

/// Bắn tia từ `o` theo hướng đơn vị `d`, tìm khối đặc đầu tiên trong tầm `max_dist`.
/// (Duyệt lưới voxel kiểu Amanatides-Woo.) Mắt nằm trong khối đặc thì trả về None.
pub fn raycast(world: &World, o: [f32; 3], d: [f32; 3], max_dist: f32) -> Option<Hit> {
    let mut cell = [o[0].floor() as i32, o[1].floor() as i32, o[2].floor() as i32];
    if world.is_solid(cell[0], cell[1], cell[2]) {
        return None;
    }
    let mut step = [0i32; 3];
    let mut t_max = [f32::INFINITY; 3];
    let mut t_delta = [f32::INFINITY; 3];
    for i in 0..3 {
        if d[i] > 0.0 {
            step[i] = 1;
            t_delta[i] = 1.0 / d[i];
            t_max[i] = ((cell[i] as f32 + 1.0) - o[i]) / d[i];
        } else if d[i] < 0.0 {
            step[i] = -1;
            t_delta[i] = -1.0 / d[i];
            t_max[i] = (o[i] - cell[i] as f32) / -d[i];
        }
    }
    loop {
        let axis = if t_max[0] < t_max[1] {
            if t_max[0] < t_max[2] {
                0
            } else {
                2
            }
        } else if t_max[1] < t_max[2] {
            1
        } else {
            2
        };
        if t_max[axis] > max_dist {
            return None;
        }
        let prev = cell;
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        if world.is_solid(cell[0], cell[1], cell[2]) {
            return Some(Hit { block: cell, prev });
        }
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
    use crate::world::STONE;

    const W: f32 = 2000.0;
    const H: f32 = 1000.0;
    /// Một điểm trống trên màn hình (không trúng nút nào) ở nửa phải.
    const FREE_RIGHT: (f32, f32) = (1500.0, 300.0);
    const TOP: i32 = 30; // mặt phẳng thử nghiệm: khối cao nhất ở y = 30, đứng ở y = 31

    fn transform(m: &Mat4, p: [f32; 3]) -> [f32; 4] {
        let mut o = [0.0; 4];
        for r in 0..4 {
            o[r] = m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r];
        }
        o
    }

    /// Sân phẳng x -4..=8, z -4..=4, mặt ở y = TOP; chỗ khác vẫn là địa hình thật.
    fn arena() -> (World, Game) {
        let mut w = World::new();
        for x in -4..=8 {
            for z in -4..=4 {
                for y in 0..CH {
                    w.set(x, y, z, if y <= TOP { STONE } else { AIR });
                }
            }
        }
        let mut g = Game::new();
        g.set_screen(W, H);
        g.pos = [0.5, TOP as f32 + 1.0 + SKIN, 0.5];
        (w, g)
    }

    fn run(g: &mut Game, w: &mut World, frames: usize) {
        for _ in 0..frames {
            g.update(1.0 / 60.0, w);
        }
    }

    #[test]
    fn projection_conventions() {
        let mut g = Game::new();
        g.pos = [0.0, -EYE_HEIGHT, 0.0]; // mắt ở gốc toạ độ
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
        g.pos = [0.0, -EYE_HEIGHT, 0.0];
        g.pitch = 0.0;
        g.yaw = std::f32::consts::FRAC_PI_2; // quay phải 90 độ: nhìn về +X
        let vp = g.view_proj(1.0);
        let c = transform(&vp, [5.0, 0.0, 0.0]);
        assert!(c[0].abs() < 1e-3 && c[3] > 0.0);
    }

    #[test]
    fn raycast_hits_expected_block() {
        let (w, _) = arena();
        // Từ trên cao bắn thẳng xuống: trúng mặt trên của sân, ô trước nó nằm ngay phía trên.
        let h = raycast(&w, [0.5, 40.5, 0.5], [0.0, -1.0, 0.0], 20.0).unwrap();
        assert_eq!(h.block, [0, TOP, 0]);
        assert_eq!(h.prev, [0, TOP + 1, 0]);
        // Quá tầm thì không trúng.
        assert!(raycast(&w, [0.5, 40.5, 0.5], [0.0, -1.0, 0.0], 5.0).is_none());
        // Bắn lên trời thì không trúng gì.
        assert!(raycast(&w, [0.5, 40.5, 0.5], [0.0, 1.0, 0.0], 50.0).is_none());
        // Bắn ngang vào một bức tường: ô trước là ô phía -x của tường.
        let mut w = w;
        w.set(3, TOP + 1, 0, STONE);
        let h = raycast(&w, [0.5, TOP as f32 + 1.5, 0.5], [1.0, 0.0, 0.0], 10.0).unwrap();
        assert_eq!(h.block, [3, TOP + 1, 0]);
        assert_eq!(h.prev, [2, TOP + 1, 0]);
        // Hướng chéo, toạ độ âm.
        let d = {
            let (a, b, c) = (-1.0f32, -0.5f32, -1.0f32);
            let l = (a * a + b * b + c * c).sqrt();
            [a / l, b / l, c / l]
        };
        let h = raycast(&w, [0.5, TOP as f32 + 2.5, 0.5], d, 20.0).unwrap();
        assert!(h.block[1] <= TOP + 1);
        // Mắt nằm trong khối đặc.
        assert!(raycast(&w, [0.5, 10.5, 0.5], [0.0, 1.0, 0.0], 5.0).is_none());
    }

    #[test]
    fn falls_and_lands_on_ground() {
        let (mut w, mut g) = arena();
        g.pos[1] = 55.0;
        run(&mut g, &mut w, 180);
        assert!(g.on_ground);
        assert!((g.pos[1] - (TOP as f32 + 1.0)).abs() < 0.01, "y={}", g.pos[1]);
        // Đứng yên thì không rung.
        let y = g.pos[1];
        run(&mut g, &mut w, 60);
        assert!((g.pos[1] - y).abs() < 1e-4);
        assert!(g.on_ground);
    }

    #[test]
    fn fast_fall_does_not_tunnel() {
        let (mut w, mut g) = arena();
        g.pos[1] = 62.0;
        // Khung hình dài 0,1 giây, rơi tới tốc độ tối đa.
        for _ in 0..80 {
            g.update(0.1, &mut w);
        }
        assert!(g.pos[1] > TOP as f32 + 0.9, "xuyên sàn: y={}", g.pos[1]);
    }

    #[test]
    fn jump_goes_over_one_block_then_lands() {
        let (mut w, mut g) = arena();
        run(&mut g, &mut w, 5);
        let (cx, cy) = g.layout().jump.center();
        g.touch(7, Phase::Started, cx, cy);
        assert!(g.jump_held());
        let mut max_y = g.pos[1];
        for _ in 0..40 {
            g.update(1.0 / 60.0, &mut w);
            max_y = max_y.max(g.pos[1]);
        }
        assert!(max_y > TOP as f32 + 1.0 + 1.2, "nhảy chưa đủ cao: {max_y}");
        assert!(max_y < TOP as f32 + 1.0 + 1.7, "nhảy quá cao: {max_y}");
        g.touch(7, Phase::Ended, cx, cy);
        run(&mut g, &mut w, 90);
        assert!(g.on_ground && (g.pos[1] - (TOP as f32 + 1.0)).abs() < 0.01);
    }

    #[test]
    fn wall_blocks_walking_and_one_block_step_needs_jump() {
        let (mut w, mut g) = arena();
        run(&mut g, &mut w, 5);
        for z in -1..=1 {
            for y in TOP + 1..=TOP + 3 {
                w.set(3, y, z, STONE);
            }
        }
        g.yaw = std::f32::consts::FRAC_PI_2; // nhìn về +X
        g.pitch = 0.0;
        g.touch(1, Phase::Started, 300.0, 700.0);
        g.touch(1, Phase::Moved, 300.0, 500.0); // kéo lên = đi tới
        run(&mut g, &mut w, 180);
        let stop = 3.0 - HALF_W;
        assert!(g.pos[0] < stop + 0.01 && g.pos[0] > stop - 0.1, "x={}", g.pos[0]);
        assert!((g.pos[2] - 0.5).abs() < 0.01, "lệch z: {}", g.pos[2]);
        assert!((g.pos[1] - (TOP as f32 + 1.0)).abs() < 0.01, "không được trèo: y={}", g.pos[1]);
    }

    #[test]
    fn sliding_along_wall() {
        let (mut w, mut g) = arena();
        run(&mut g, &mut w, 5);
        for z in -4..=4 {
            for y in TOP + 1..=TOP + 3 {
                w.set(2, y, z, STONE);
            }
        }
        // Đi chéo về phía tường (+x và -z): bị chặn theo x nhưng vẫn trượt theo z.
        g.yaw = std::f32::consts::FRAC_PI_4;
        g.pitch = 0.0;
        g.touch(1, Phase::Started, 300.0, 700.0);
        g.touch(1, Phase::Moved, 300.0, 500.0);
        run(&mut g, &mut w, 40);
        assert!(g.pos[0] < 2.0 - HALF_W + 0.01);
        assert!(g.pos[2] < -1.0, "phải trượt dọc tường: z={}", g.pos[2]);
    }

    #[test]
    fn stick_moves_in_flight_and_stops_on_release() {
        let (mut w, mut g) = arena();
        g.flying = true;
        g.pos[1] = 45.0;
        g.pitch = 0.0;
        let (x0, z0) = (g.pos[0], g.pos[2]);
        g.touch(1, Phase::Started, 300.0, 700.0);
        g.touch(1, Phase::Moved, 300.0, 500.0);
        run(&mut g, &mut w, 30);
        assert!(g.pos[2] < z0 - 3.0, "đi tới phải giảm z: {}", g.pos[2]);
        assert!((g.pos[0] - x0).abs() < 0.01);
        assert!((g.pos[1] - 45.0).abs() < 0.01, "pitch 0 thì không đổi độ cao");
        g.touch(1, Phase::Ended, 300.0, 500.0);
        let z1 = g.pos[2];
        run(&mut g, &mut w, 10);
        assert!((g.pos[2] - z1).abs() < 1e-6, "thả tay thì đứng yên");
        assert!(g.stick_visual().is_none());
    }

    #[test]
    fn flying_up_with_jump_button_and_down_by_looking() {
        let (mut w, mut g) = arena();
        g.flying = true;
        g.pos[1] = 40.0;
        let (cx, cy) = g.layout().jump.center();
        g.touch(3, Phase::Started, cx, cy);
        run(&mut g, &mut w, 30);
        assert!(g.pos[1] > 44.0, "y={}", g.pos[1]);
        g.touch(3, Phase::Ended, cx, cy);
        let y = g.pos[1];
        run(&mut g, &mut w, 10);
        assert!((g.pos[1] - y).abs() < 1e-4, "bay: không có đầu vào thì lơ lửng");
        // Nhìn xuống rồi đi tới thì hạ độ cao.
        g.pitch = -1.0;
        g.touch(1, Phase::Started, 300.0, 700.0);
        g.touch(1, Phase::Moved, 300.0, 500.0);
        run(&mut g, &mut w, 20);
        assert!(g.pos[1] < y - 3.0);
    }

    #[test]
    fn look_drag_turns_camera_without_acting() {
        let (mut w, mut g) = arena();
        let (px, py) = FREE_RIGHT;
        g.touch(2, Phase::Started, px, py);
        g.touch(2, Phase::Moved, px + 100.0, py);
        assert!(g.yaw > 0.0);
        g.touch(2, Phase::Moved, px + 100.0, py - 100.0); // kéo lên = nhìn lên
        assert!(g.pitch > -0.2 + 0.1);
        run(&mut g, &mut w, 30); // giữ lâu cũng không đập vì đã kéo
        g.touch(2, Phase::Ended, px + 100.0, py - 100.0);
        let before = w.get(0, TOP, 0);
        let dirty = g.update(1.0 / 60.0, &mut w);
        assert!(dirty.is_empty());
        assert_eq!(w.get(0, TOP, 0), before);
    }

    #[test]
    fn small_finger_jitter_is_still_a_tap() {
        let (mut w, mut g) = arena();
        g.pitch = -0.6;
        let (px, py) = FREE_RIGHT;
        g.touch(2, Phase::Started, px, py);
        g.touch(2, Phase::Moved, px + 3.0, py - 2.0); // rung nhẹ, chưa vượt ngưỡng
        let yaw = g.yaw;
        g.touch(2, Phase::Ended, px + 3.0, py - 2.0);
        assert_eq!(g.yaw, yaw, "rung nhẹ không được xoay camera");
        let dirty = g.update(1.0 / 60.0, &mut w);
        assert!(!dirty.is_empty(), "chạm nhanh phải đặt khối");
    }

    #[test]
    fn tap_places_selected_block_on_target() {
        let (mut w, mut g) = arena();
        run(&mut g, &mut w, 5);
        g.pitch = -0.6;
        g.selected = 4; // ván gỗ
        g.update(1.0 / 60.0, &mut w);
        let hit = g.target().expect("phải nhắm trúng mặt sân");
        assert_eq!(hit.block[1], TOP);
        assert_eq!(hit.prev[1], TOP + 1);
        assert_eq!(w.get(hit.prev[0], hit.prev[1], hit.prev[2]), AIR);

        let (px, py) = FREE_RIGHT;
        g.touch(2, Phase::Started, px, py);
        g.touch(2, Phase::Ended, px, py);
        let dirty = g.update(1.0 / 60.0, &mut w);
        assert!(!dirty.is_empty());
        assert_eq!(w.get(hit.prev[0], hit.prev[1], hit.prev[2]), HOTBAR[4]);
        // Đặt xong thì không đặt thêm khi không chạm nữa.
        assert!(g.update(1.0 / 60.0, &mut w).is_empty());
    }

    #[test]
    fn long_press_breaks_block_once_per_interval_and_does_not_place() {
        let (mut w, mut g) = arena();
        run(&mut g, &mut w, 5);
        g.pitch = -0.6;
        g.update(1.0 / 60.0, &mut w);
        let first = g.target().unwrap();
        let [bx, by, bz] = first.block;
        assert_ne!(w.get(bx, by, bz), AIR);

        let (px, py) = FREE_RIGHT;
        g.touch(2, Phase::Started, px, py);
        for _ in 0..5 {
            g.update(0.1, &mut w); // tổng 0,5 giây: đúng một lần đập
        }
        assert_eq!(w.get(bx, by, bz), AIR, "khối phải bị đập");
        // Tia nhắm xiên nên sau khi đào sẽ trúng khối khác (thành hố hoặc đáy hố).
        let second = g.target().unwrap();
        assert_ne!(second.block, first.block);
        // Sau 0,5 giây giữ tay chỉ được đập đúng một khối.
        let mut holes = 0;
        for x in -4..=8 {
            for z in -4..=4 {
                for y in 0..=TOP {
                    if w.get(x, y, z) == AIR {
                        holes += 1;
                    }
                }
            }
        }
        assert_eq!(holes, 1, "mới đập một lần");

        // Thả tay sau khi đã đập thì không đặt khối.
        g.touch(2, Phase::Ended, px, py);
        let dirty = g.update(1.0 / 60.0, &mut w);
        assert!(dirty.is_empty());
        assert_eq!(w.get(bx, by, bz), AIR);
    }

    #[test]
    fn cannot_break_bedrock_layer() {
        let (mut w, mut g) = arena();
        g.pos[1] = 5.0 + SKIN;
        // Đào hết cột dưới chân trừ đáy: dùng set trực tiếp để dựng hố.
        for y in 1..=TOP {
            w.set(0, y, 0, AIR);
        }
        g.pos = [0.5, 1.0 + SKIN, 0.5];
        g.pitch = -1.5;
        g.update(1.0 / 60.0, &mut w);
        let h = g.target().expect("nhắm xuống đáy hố");
        assert_eq!(h.block[1], 0);
        let (px, py) = FREE_RIGHT;
        g.touch(2, Phase::Started, px, py);
        for _ in 0..5 {
            g.update(0.1, &mut w);
        }
        assert_ne!(w.get(0, 0, 0), AIR, "y = 0 không được đập");
    }

    #[test]
    fn cannot_place_block_inside_player() {
        let (mut w, mut g) = arena();
        run(&mut g, &mut w, 5);
        g.pitch = -1.5; // nhìn gần như thẳng xuống chân
        g.update(1.0 / 60.0, &mut w);
        let hit = g.target().unwrap();
        assert_eq!(hit.prev, [0, TOP + 1, 0]);
        let (px, py) = FREE_RIGHT;
        g.touch(2, Phase::Started, px, py);
        g.touch(2, Phase::Ended, px, py);
        let dirty = g.update(1.0 / 60.0, &mut w);
        assert!(dirty.is_empty());
        assert_eq!(w.get(0, TOP + 1, 0), AIR);
    }

    #[test]
    fn hotbar_and_fly_buttons() {
        let (mut w, mut g) = arena();
        for i in 0..crate::world::HOTBAR_SLOTS {
            let (cx, cy) = g.layout().hotbar[i].center();
            g.touch(10 + i as u64, Phase::Started, cx, cy);
            g.touch(10 + i as u64, Phase::Ended, cx, cy);
            assert_eq!(g.selected, i);
        }
        // Chạm vào ô thanh vật phẩm không được tính là đặt khối hay điều khiển.
        assert!(g.update(1.0 / 60.0, &mut w).is_empty());
        assert!(g.stick_visual().is_none());

        assert!(!g.flying);
        let (fx, fy) = g.layout().fly.center();
        g.touch(20, Phase::Started, fx, fy);
        g.touch(20, Phase::Ended, fx, fy);
        assert!(g.flying);
        g.touch(21, Phase::Started, fx, fy);
        g.touch(21, Phase::Ended, fx, fy);
        assert!(!g.flying);
    }

    #[test]
    fn spawn_is_on_the_surface_and_walkable() {
        let mut w = World::new();
        let mut g = Game::new();
        g.set_screen(W, H);
        for _ in 0..120 {
            g.update(1.0 / 60.0, &mut w);
        }
        assert!(g.on_ground, "y={}", g.pos[1]);
        assert!((g.pos[1] - (height_at(0, 0) as f32 + 1.0)).abs() < 0.05);
    }
}
