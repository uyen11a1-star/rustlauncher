//! Vật lý người chơi giống bản Java: bước 20 tick/giây, trọng lực 0,08, ma sát, nhảy 0,42,
//! bước lên bậc cao 0,6, ngồi không rơi khỏi mép, bơi, leo thang, bay (creative).
use crate::{blocks::Shape, world::{World, UNKNOWN}};

const EPS: f64 = 1e-7;
pub const TICK: f32 = 0.05;
const HALF_W: f64 = 0.3;
const STEP: f64 = 0.6;

type Box6 = [f64; 6];

/// Thế giới đã nạp (bản sao phía luồng vẽ) kèm hình dạng va chạm của từng block state.
pub struct Collider {
    pub world: World,
    pub shapes: Vec<Shape>,
}

impl Collider {
    fn shape(&self, x: i32, y: i32, z: i32) -> Option<&Shape> {
        let id = self.world.get(x, y, z);
        if id == 0 || id == UNKNOWN {
            return None;
        }
        self.shapes.get(id as usize)
    }

    pub fn fluid_at(&self, x: i32, y: i32, z: i32) -> u8 {
        self.shape(x, y, z).map_or(0, |s| s.fluid)
    }

    pub fn climb_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.shape(x, y, z).map_or(false, |s| s.climb)
    }

    /// Chunk chứa ô này đã nạp chưa.
    pub fn loaded(&self, x: i32, z: i32) -> bool {
        self.world.chunks.contains_key(&(x >> 4, z >> 4))
    }

    /// Các hộp va chạm (tọa độ thế giới) chạm vùng `r`. Chunk chưa nạp coi như đặc.
    fn collect(&self, r: Box6, out: &mut Vec<Box6>) {
        let (x0, x1) = (r[0].floor() as i32, r[3].floor() as i32);
        let (y0, y1) = (r[1].floor() as i32, r[4].floor() as i32);
        let (z0, z1) = (r[2].floor() as i32, r[5].floor() as i32);
        for x in x0..=x1 {
            for z in z0..=z1 {
                for y in y0..=y1 {
                    let id = self.world.get(x, y, z);
                    if id == UNKNOWN {
                        out.push([x as f64, y as f64, z as f64, x as f64 + 1.0, y as f64 + 1.0, z as f64 + 1.0]);
                    } else if id != 0 {
                        if let Some(s) = self.shapes.get(id as usize) {
                            for b in &s.boxes {
                                out.push([x as f64 + b[0] as f64, y as f64 + b[1] as f64, z as f64 + b[2] as f64, x as f64 + b[3] as f64, y as f64 + b[4] as f64, z as f64 + b[5] as f64]);
                            }
                        }
                    }
                }
            }
        }
    }

    fn intersects(&self, b: Box6) -> bool {
        let mut obs = Vec::new();
        self.collect(b, &mut obs);
        obs.iter().any(|o| (0..3).all(|a| b[a + 3] > o[a] + EPS && b[a] < o[a + 3] - EPS))
    }
}

fn shift(b: Box6, a: usize, d: f64) -> Box6 {
    let mut r = b;
    r[a] += d;
    r[a + 3] += d;
    r
}

fn shift3(b: Box6, d: [f64; 3]) -> Box6 {
    [b[0] + d[0], b[1] + d[1], b[2] + d[2], b[3] + d[0], b[4] + d[1], b[5] + d[2]]
}

/// Giới hạn dịch chuyển `d` theo trục `a` để hộp `b` không xuyên các vật cản.
fn clip(obs: &[Box6], b: &Box6, a: usize, mut d: f64) -> f64 {
    if d == 0.0 {
        return 0.0;
    }
    let (o1, o2) = ((a + 1) % 3, (a + 2) % 3);
    for o in obs {
        if b[o1 + 3] > o[o1] + EPS && b[o1] < o[o1 + 3] - EPS && b[o2 + 3] > o[o2] + EPS && b[o2] < o[o2 + 3] - EPS {
            if d > 0.0 && b[a + 3] <= o[a] + EPS {
                d = d.min(o[a] - b[a + 3]);
            } else if d < 0.0 && b[a] >= o[a + 3] - EPS {
                d = d.max(o[a + 3] - b[a]);
            }
        }
    }
    d
}

/// Dịch chuyển hộp theo `mv`: trục Y trước, rồi X/Z (trục dài hơn trước... như Java).
fn collide_axes(c: &Collider, bb: Box6, mv: [f64; 3]) -> [f64; 3] {
    let mut reg = bb;
    for a in 0..3 {
        if mv[a] < 0.0 {
            reg[a] += mv[a];
        } else {
            reg[a + 3] += mv[a];
        }
    }
    let mut obs = Vec::new();
    c.collect(reg, &mut obs);
    let mut b = bb;
    let mut r = [0.0; 3];
    r[1] = clip(&obs, &b, 1, mv[1]);
    b = shift(b, 1, r[1]);
    if mv[0].abs() < mv[2].abs() {
        r[2] = clip(&obs, &b, 2, mv[2]);
        b = shift(b, 2, r[2]);
        r[0] = clip(&obs, &b, 0, mv[0]);
    } else {
        r[0] = clip(&obs, &b, 0, mv[0]);
        b = shift(b, 0, r[0]);
        r[2] = clip(&obs, &b, 2, mv[2]);
    }
    r
}

fn h2(v: [f64; 3]) -> f64 {
    v[0] * v[0] + v[2] * v[2]
}

/// Như collide_axes nhưng có bước lên bậc cao tới 0,6 (thuật toán của Entity.collide).
fn collide(c: &Collider, bb: Box6, mv: [f64; 3], on_ground: bool) -> [f64; 3] {
    let r = collide_axes(c, bb, mv);
    let (fx, fz, fy) = ((mv[0] - r[0]).abs() > EPS, (mv[2] - r[2]).abs() > EPS, (mv[1] - r[1]).abs() > EPS);
    let grounded = on_ground || (fy && mv[1] < 0.0);
    if grounded && (fx || fz) {
        let mut v1 = collide_axes(c, bb, [mv[0], STEP, mv[2]]);
        let mut wide = bb;
        for &a in &[0usize, 2] {
            if mv[a] < 0.0 {
                wide[a] += mv[a];
            } else {
                wide[a + 3] += mv[a];
            }
        }
        let v2 = collide_axes(c, wide, [0.0, STEP, 0.0]);
        if v2[1] < STEP {
            let v3 = collide_axes(c, shift3(bb, v2), [mv[0], 0.0, mv[2]]);
            let v3 = [v3[0] + v2[0], v3[1] + v2[1], v3[2] + v2[2]];
            if h2(v3) > h2(v1) {
                v1 = v3;
            }
        }
        if h2(v1) > h2(r) {
            let d = collide_axes(c, shift3(bb, v1), [0.0, -v1[1] + mv[1], 0.0]);
            return [v1[0] + d[0], v1[1] + d[1], v1[2] + d[2]];
        }
    }
    r
}

#[derive(Default, Clone, Copy)]
pub struct Input {
    pub fwd: f64,
    pub strafe: f64,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
}

pub struct Player {
    /// vị trí chân (giữa đáy hộp)
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub on_ground: bool,
    pub fly: bool,
    pub sneaking: bool,
    pub sprinting: bool,
    pub in_water: bool,
    pub hcol: bool,
    jump_delay: u32,
}

impl Player {
    pub fn new(pos: [f64; 3], fly: bool) -> Player {
        Player { pos, vel: [0.0; 3], on_ground: false, fly, sneaking: false, sprinting: false, in_water: false, hcol: false, jump_delay: 0 }
    }

    pub fn eye_height(&self) -> f64 {
        if self.sneaking { 1.27 } else { 1.62 }
    }

    fn height(&self) -> f64 {
        if self.sneaking { 1.5 } else { 1.8 }
    }

    fn bb(&self) -> Box6 {
        [self.pos[0] - HALF_W, self.pos[1], self.pos[2] - HALF_W, self.pos[0] + HALF_W, self.pos[1] + self.height(), self.pos[2] + HALF_W]
    }

    fn move_relative(&mut self, speed: f64, f: f64, s: f64, yaw: f64) {
        let d = f * f + s * s;
        if d < 1e-7 {
            return;
        }
        let k = if d > 1.0 { speed / d.sqrt() } else { speed };
        let (f, s) = (f * k, s * k);
        let (sn, cs) = yaw.sin_cos();
        // hướng tiến = (-sin, -cos), sang phải = (cos, -sin)
        self.vel[0] += -sn * f + cs * s;
        self.vel[2] += -cs * f - sn * s;
    }

    /// Di chuyển theo vận tốc hiện tại, xử lý va chạm, cập nhật on_ground và triệt tiêu vận tốc ở trục va.
    fn step_move(&mut self, col: Option<&Collider>) {
        let mut mv = self.vel;
        if let Some(c) = col {
            if self.sneaking && self.on_ground {
                let bb = self.bb();
                let free = |dx: f64, dz: f64| !c.intersects(shift3(bb, [dx, -STEP, dz]));
                let adj = |v: &mut f64| {
                    if v.abs() < 0.05 {
                        *v = 0.0;
                    } else if *v > 0.0 {
                        *v -= 0.05;
                    } else {
                        *v += 0.05;
                    }
                };
                while mv[0] != 0.0 && free(mv[0], 0.0) {
                    adj(&mut mv[0]);
                }
                while mv[2] != 0.0 && free(0.0, mv[2]) {
                    adj(&mut mv[2]);
                }
                while mv[0] != 0.0 && mv[2] != 0.0 && free(mv[0], mv[2]) {
                    adj(&mut mv[0]);
                    adj(&mut mv[2]);
                }
            }
        }
        let r = match col {
            Some(c) => collide(c, self.bb(), mv, self.on_ground),
            None => mv,
        };
        for a in 0..3 {
            self.pos[a] += r[a];
        }
        let (cx, cy, cz) = ((mv[0] - r[0]).abs() > EPS, (mv[1] - r[1]).abs() > EPS, (mv[2] - r[2]).abs() > EPS);
        self.hcol = cx || cz;
        self.on_ground = cy && mv[1] < 0.0;
        if cx {
            self.vel[0] = 0.0;
        }
        if cz {
            self.vel[2] = 0.0;
        }
        if cy {
            self.vel[1] = 0.0;
        }
    }

    /// Một tick (1/20 giây). `yaw` theo quy ước của màn hình xem.
    pub fn tick(&mut self, inp: &Input, yaw: f64, pitch: f64, col: Option<&Collider>) {
        let (bx, by, bz) = (self.pos[0].floor() as i32, self.pos[1].floor() as i32, self.pos[2].floor() as i32);
        let (mut water, mut lava, mut climbing) = (false, false, false);
        if let Some(c) = col {
            let f = c.fluid_at(bx, (self.pos[1] + 0.3).floor() as i32, bz);
            water = f == 1;
            lava = f == 2;
            climbing = c.climb_at(bx, by, bz);
        }
        self.in_water = water;
        self.sneaking = inp.sneak && !self.fly;
        self.sprinting = inp.sprint && inp.fwd > 0.8 && !self.sneaking;
        let (mut f, mut s) = (inp.fwd * 0.98, inp.strafe * 0.98);
        if self.sneaking {
            f *= 0.3;
            s *= 0.3;
        }
        if self.jump_delay > 0 {
            self.jump_delay -= 1;
        }
        if !inp.jump {
            self.jump_delay = 0;
        }

        if self.fly {
            self.vel[1] += 0.15 * (inp.jump as i32 as f64 - inp.sneak as i32 as f64);
            let sp = if self.sprinting { 0.1 } else { 0.05 };
            self.move_relative(sp, f, s, yaw);
            self.step_move(col);
            self.vel[1] *= 0.6;
            let fr = if self.on_ground { 0.546 } else { 0.91 };
            self.vel[0] *= fr;
            self.vel[2] *= fr;
            if self.on_ground && !inp.jump && col.is_some() {
                self.fly = false; // chạm đất thì thôi bay
            }
        } else if water || lava {
            if water && self.sprinting {
                // bơi nhanh: vận tốc đứng nhích dần về hướng nhìn (nhìn lên thì bơi lên, nhìn xuống thì lặn)
                let ly = pitch.sin();
                let k = if ly < -0.2 { 0.085 } else { 0.06 };
                let head_in_fluid = col.map_or(false, |c| c.fluid_at(bx, (self.pos[1] + 0.9).floor() as i32, bz) != 0);
                if ly <= 0.0 || inp.jump || head_in_fluid {
                    self.vel[1] += (ly - self.vel[1]) * k;
                }
            }
            if inp.jump {
                self.vel[1] += 0.04;
            }
            self.move_relative(0.02, f, s, yaw);
            let old_y = self.pos[1];
            self.step_move(col);
            if water {
                let d = if self.sprinting { 0.9 } else { 0.8 };
                self.vel[0] *= d;
                self.vel[1] *= 0.8;
                self.vel[2] *= d;
                if !self.sprinting {
                    self.vel[1] -= 0.005; // trọng lực dưới nước = 0,08 / 16 (bản mới)
                }
            } else {
                for v in self.vel.iter_mut() {
                    *v *= 0.5;
                }
                self.vel[1] -= 0.02;
            }
            // trèo lên bờ: đang đụng khối bên cạnh mà phía trên 0,6 khối còn trống thì được đẩy lên 0,3 (như Java)
            if self.hcol {
                if let Some(c) = col {
                    let probe = shift3(self.bb(), [self.vel[0], self.vel[1] + 0.6 - self.pos[1] + old_y, self.vel[2]]);
                    if !c.intersects(probe) {
                        self.vel[1] = 0.3;
                    }
                }
            }
        } else {
            if inp.jump && self.on_ground && self.jump_delay == 0 {
                self.vel[1] = 0.42;
                if self.sprinting {
                    let (sn, cs) = yaw.sin_cos();
                    self.vel[0] += -sn * 0.2;
                    self.vel[2] += -cs * 0.2;
                }
                self.jump_delay = 10;
            }
            let fr = if self.on_ground { 0.546 } else { 0.91 };
            let base = 0.1 * if self.sprinting { 1.3 } else { 1.0 };
            let sp = if self.on_ground { base * 0.162_771_36 / (fr * fr * fr) } else if self.sprinting { 0.026 } else { 0.02 };
            self.move_relative(sp, f, s, yaw);
            if climbing {
                self.vel[0] = self.vel[0].clamp(-0.15, 0.15);
                self.vel[2] = self.vel[2].clamp(-0.15, 0.15);
                self.vel[1] = self.vel[1].max(-0.15);
                if self.sneaking && self.vel[1] < 0.0 {
                    self.vel[1] = 0.0;
                }
            }
            self.step_move(col);
            if climbing && self.hcol {
                self.vel[1] = 0.2;
            }
            self.vel[1] = (self.vel[1] - 0.08) * 0.98;
            self.vel[0] *= fr;
            self.vel[2] *= fr;
        }
    }
}
