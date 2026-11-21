//! Quần xã thế giới thường 1.21: dựng bảng tham số (cách Minecraft chia quần xã theo khí hậu) rồi tra gần nhất.
//! Bảng được dựng lại bằng quy tắc và đối chiếu từng mục với dữ liệu tham chiếu của bản gốc.
type P = (i64, i64);

pub struct Entry {
    r: [P; 7],
    biome: &'static str,
}

const FULL: P = (-10000, 10000);
const TB: [P; 5] = [(-10000, -4500), (-4500, -1500), (-1500, 2000), (2000, 5500), (5500, 10000)];
const HB: [P; 5] = [(-10000, -3500), (-3500, -1000), (-1000, 1000), (1000, 3000), (3000, 10000)];
const CB: [i64; 5] = [-1900, -1100, 300, 3000, 10000];
const EB: [i64; 8] = [-10000, -7799, -3750, -2225, 500, 4500, 5500, 10000];
// (khoảng độ kỳ quặc, loại lát: 0 giữa, 1 cao, 2 đỉnh, 3 thấp, 4 thung lũng)
const SLICES: [(P, u8); 13] = [
    ((-10000, -9333), 0),
    ((-9333, -7666), 1),
    ((-7666, -5666), 2),
    ((-5666, -4000), 1),
    ((-4000, -2666), 0),
    ((-2666, -500), 3),
    ((-500, 500), 4),
    ((500, 2666), 3),
    ((2666, 4000), 0),
    ((4000, 5666), 1),
    ((5666, 7666), 2),
    ((7666, 9333), 1),
    ((9333, 10000), 0),
];

const MID: [[&str; 5]; 5] = [
    ["snowy_plains", "snowy_plains", "snowy_plains", "snowy_taiga", "taiga"],
    ["plains", "plains", "forest", "taiga", "old_growth_spruce_taiga"],
    ["flower_forest", "plains", "forest", "birch_forest", "dark_forest"],
    ["savanna", "savanna", "forest", "jungle", "jungle"],
    ["desert", "desert", "desert", "desert", "desert"],
];
const MIDV: [[&str; 5]; 5] = [
    ["ice_spikes", "", "snowy_taiga", "", ""],
    ["", "", "", "", "old_growth_pine_taiga"],
    ["sunflower_plains", "", "", "old_growth_birch_forest", ""],
    ["", "", "plains", "sparse_jungle", "bamboo_jungle"],
    ["", "", "", "", ""],
];
const PLA: [[&str; 5]; 5] = [
    ["snowy_plains", "snowy_plains", "snowy_plains", "snowy_taiga", "snowy_taiga"],
    ["meadow", "meadow", "forest", "taiga", "old_growth_spruce_taiga"],
    ["meadow", "meadow", "meadow", "meadow", "pale_garden"],
    ["savanna_plateau", "savanna_plateau", "forest", "forest", "jungle"],
    ["badlands", "badlands", "badlands", "wooded_badlands", "wooded_badlands"],
];
const PLAV: [[&str; 5]; 5] = [
    ["ice_spikes", "", "", "", ""],
    ["cherry_grove", "", "meadow", "meadow", "old_growth_pine_taiga"],
    ["cherry_grove", "cherry_grove", "forest", "birch_forest", ""],
    ["", "", "", "", ""],
    ["eroded_badlands", "eroded_badlands", "", "", ""],
];
const SHA: [[&str; 5]; 5] = [
    ["windswept_gravelly_hills", "windswept_gravelly_hills", "windswept_hills", "windswept_forest", "windswept_forest"],
    ["windswept_gravelly_hills", "windswept_gravelly_hills", "windswept_hills", "windswept_forest", "windswept_forest"],
    ["windswept_hills", "windswept_hills", "windswept_hills", "windswept_forest", "windswept_forest"],
    ["", "", "", "", ""],
    ["", "", "", "", ""],
];

fn middle(t: usize, h: usize, neg: bool) -> &'static str {
    if neg || MIDV[t][h].is_empty() {
        MID[t][h]
    } else {
        MIDV[t][h]
    }
}

fn badlands(h: usize, neg: bool) -> &'static str {
    if h < 2 {
        if neg {
            "badlands"
        } else {
            "eroded_badlands"
        }
    } else if h < 3 {
        "badlands"
    } else {
        "wooded_badlands"
    }
}

fn mid_badl(t: usize, h: usize, neg: bool) -> &'static str {
    if t == 4 {
        badlands(h, neg)
    } else {
        middle(t, h, neg)
    }
}

fn plateau(t: usize, h: usize, neg: bool) -> &'static str {
    if neg || PLAV[t][h].is_empty() {
        PLA[t][h]
    } else {
        PLAV[t][h]
    }
}

fn slope(t: usize, h: usize, neg: bool) -> &'static str {
    if t >= 3 {
        plateau(t, h, neg)
    } else if h <= 1 {
        "snowy_slopes"
    } else {
        "grove"
    }
}

fn shattered(t: usize, h: usize, neg: bool) -> &'static str {
    if SHA[t][h].is_empty() {
        middle(t, h, neg)
    } else {
        SHA[t][h]
    }
}

fn wind(t: usize, h: usize, neg: bool, fallback: &'static str) -> &'static str {
    if t > 1 && h < 4 && !neg {
        "windswept_savanna"
    } else {
        fallback
    }
}

fn peak(t: usize, h: usize, neg: bool) -> &'static str {
    if t <= 2 {
        if neg {
            "jagged_peaks"
        } else {
            "frozen_peaks"
        }
    } else if t == 3 {
        "stony_peaks"
    } else {
        badlands(h, neg)
    }
}

fn beach(t: usize) -> &'static str {
    match t {
        0 => "snowy_beach",
        4 => "desert",
        _ => "beach",
    }
}

#[derive(Clone, Copy)]
enum K {
    Const(&'static str),
    Middle,
    MidBadl,
    MidBadlSlope,
    Plateau,
    Slope,
    Shattered,
    ShatWind,
    Peak,
    Beach,
    ShatCoast,
    Swamp,
    PlaSlopeCold,
    River,
    SwampValley,
}

use K::*;
// (lục địa từ, đến, xói mòn từ, đến, cách chọn)
type R = (usize, usize, usize, usize, K);

const MID_NEG: &[R] = &[
    (0, 0, 0, 2, Const("stony_shore")),
    (0, 0, 4, 4, Beach),
    (0, 0, 5, 5, Beach),
    (0, 0, 6, 6, Beach),
    (0, 1, 3, 3, Middle),
    (1, 1, 2, 2, Middle),
    (1, 1, 5, 5, Middle),
    (1, 2, 1, 1, MidBadlSlope),
    (1, 3, 0, 0, Slope),
    (1, 3, 4, 4, Middle),
    (1, 3, 6, 6, Swamp),
    (2, 2, 2, 2, MidBadl),
    (2, 3, 3, 3, MidBadl),
    (2, 3, 5, 5, Shattered),
    (3, 3, 1, 1, PlaSlopeCold),
    (3, 3, 2, 2, Plateau),
];
const MID_POS: &[R] = &[
    (0, 0, 0, 2, Const("stony_shore")),
    (0, 0, 5, 5, ShatCoast),
    (0, 0, 6, 6, Middle),
    (0, 1, 3, 3, Middle),
    (0, 3, 4, 4, Middle),
    (1, 1, 2, 2, Middle),
    (1, 1, 5, 5, ShatCoast),
    (1, 2, 1, 1, MidBadlSlope),
    (1, 3, 0, 0, Slope),
    (1, 3, 6, 6, Swamp),
    (2, 2, 2, 2, MidBadl),
    (2, 3, 3, 3, MidBadl),
    (2, 3, 5, 5, Shattered),
    (3, 3, 1, 1, PlaSlopeCold),
    (3, 3, 2, 2, Plateau),
];
const HIGH_NEG: &[R] = &[
    (0, 0, 0, 1, Middle),
    (0, 1, 2, 3, Middle),
    (0, 1, 5, 5, Middle),
    (0, 3, 4, 4, Middle),
    (0, 3, 6, 6, Middle),
    (1, 1, 0, 0, Slope),
    (1, 1, 1, 1, MidBadlSlope),
    (2, 2, 3, 3, MidBadl),
    (2, 3, 0, 0, Peak),
    (2, 3, 1, 1, Slope),
    (2, 3, 2, 2, Plateau),
    (2, 3, 5, 5, Shattered),
    (3, 3, 3, 3, Plateau),
];
const HIGH_POS: &[R] = &[
    (0, 0, 0, 1, Middle),
    (0, 1, 2, 3, Middle),
    (0, 1, 5, 5, ShatCoast),
    (0, 3, 4, 4, Middle),
    (0, 3, 6, 6, Middle),
    (1, 1, 0, 0, Slope),
    (1, 1, 1, 1, MidBadlSlope),
    (2, 2, 3, 3, MidBadl),
    (2, 3, 0, 0, Peak),
    (2, 3, 1, 1, Slope),
    (2, 3, 2, 2, Plateau),
    (2, 3, 5, 5, Shattered),
    (3, 3, 3, 3, Plateau),
];
const PEAKS_NEG: &[R] = &[
    (0, 1, 1, 1, MidBadlSlope),
    (0, 1, 2, 3, Middle),
    (0, 1, 5, 5, Shattered),
    (0, 3, 0, 0, Peak),
    (0, 3, 4, 4, Middle),
    (0, 3, 6, 6, Middle),
    (2, 2, 3, 3, MidBadl),
    (2, 3, 1, 1, Peak),
    (2, 3, 2, 2, Plateau),
    (2, 3, 5, 5, Shattered),
    (3, 3, 3, 3, Plateau),
];
const PEAKS_POS: &[R] = &[
    (0, 1, 1, 1, MidBadlSlope),
    (0, 1, 2, 3, Middle),
    (0, 1, 5, 5, ShatWind),
    (0, 3, 0, 0, Peak),
    (0, 3, 4, 4, Middle),
    (0, 3, 6, 6, Middle),
    (2, 2, 3, 3, MidBadl),
    (2, 3, 1, 1, Peak),
    (2, 3, 2, 2, Plateau),
    (2, 3, 5, 5, Shattered),
    (3, 3, 3, 3, Plateau),
];
const LOW_NEG: &[R] = &[
    (0, 0, 0, 2, Const("stony_shore")),
    (0, 0, 3, 4, Beach),
    (0, 0, 5, 5, Beach),
    (0, 0, 6, 6, Beach),
    (1, 1, 0, 1, MidBadl),
    (1, 1, 2, 3, Middle),
    (1, 1, 5, 5, Middle),
    (1, 3, 4, 4, Middle),
    (1, 3, 6, 6, Swamp),
    (2, 3, 0, 1, MidBadlSlope),
    (2, 3, 2, 3, MidBadl),
    (2, 3, 5, 5, Middle),
];
const LOW_POS: &[R] = &[
    (0, 0, 0, 2, Const("stony_shore")),
    (0, 0, 3, 4, Beach),
    (0, 0, 5, 5, ShatCoast),
    (0, 0, 6, 6, Beach),
    (1, 1, 0, 1, MidBadl),
    (1, 1, 2, 3, Middle),
    (1, 1, 5, 5, ShatCoast),
    (1, 3, 4, 4, Middle),
    (1, 3, 6, 6, Swamp),
    (2, 3, 0, 1, MidBadlSlope),
    (2, 3, 2, 3, MidBadl),
    (2, 3, 5, 5, Middle),
];
const VALLEY: &[R] = &[
    (0, 0, 0, 1, River),
    (0, 0, 6, 6, River),
    (0, 3, 2, 5, River),
    (1, 1, 0, 1, River),
    (1, 3, 6, 6, SwampValley),
    (2, 3, 0, 1, MidBadl),
];

fn rules(kind: u8, neg: bool) -> &'static [R] {
    match (kind, neg) {
        (0, true) => MID_NEG,
        (0, false) => MID_POS,
        (1, true) => HIGH_NEG,
        (1, false) => HIGH_POS,
        (2, true) => PEAKS_NEG,
        (2, false) => PEAKS_POS,
        (3, true) => LOW_NEG,
        (3, false) => LOW_POS,
        _ => VALLEY,
    }
}

fn pick(k: K, t: usize, h: usize, neg: bool) -> &'static str {
    match k {
        Const(b) => b,
        Middle => middle(t, h, neg),
        MidBadl => mid_badl(t, h, neg),
        MidBadlSlope => {
            if t == 0 {
                slope(t, h, neg)
            } else {
                mid_badl(t, h, neg)
            }
        }
        Plateau => plateau(t, h, neg),
        Slope => slope(t, h, neg),
        Shattered => shattered(t, h, neg),
        ShatWind => wind(t, h, neg, shattered(t, h, neg)),
        Peak => peak(t, h, neg),
        Beach => beach(t),
        ShatCoast => wind(t, h, neg, if neg { beach(t) } else { middle(t, h, neg) }),
        PlaSlopeCold => {
            if t == 0 {
                slope(t, h, neg)
            } else {
                plateau(t, h, neg)
            }
        }
        Swamp | River | SwampValley => middle(t, h, neg),
    }
}

pub struct BiomeSource {
    entries: Vec<Entry>,
}

impl BiomeSource {
    pub fn new() -> BiomeSource {
        let mut v: Vec<Entry> = Vec::new();
        // mỗi mục bề mặt có hai bản: độ sâu 0 và độ sâu 1
        let mut add = |t: P, h: P, c: P, e: P, w: P, biome: &'static str| {
            for d in [0, 10000] {
                v.push(Entry { r: [t, h, c, e, (d, d), w, (0, 0)], biome });
            }
        };
        add(FULL, FULL, (-12000, -10500), FULL, FULL, "mushroom_fields");
        let deep = ["deep_frozen_ocean", "deep_cold_ocean", "deep_ocean", "deep_lukewarm_ocean", "warm_ocean"];
        let ocean = ["frozen_ocean", "cold_ocean", "ocean", "lukewarm_ocean", "warm_ocean"];
        for i in 0..5 {
            add(TB[i], FULL, (-10500, -4550), FULL, FULL, deep[i]);
            add(TB[i], FULL, (-4550, -1900), FULL, FULL, ocean[i]);
        }
        for (w, kind) in SLICES {
            let neg = w.1 < 0;
            for &(c0, c1, e0, e1, k) in rules(kind, neg) {
                let c = (CB[c0], CB[c1 + 1]);
                let e = (EB[e0], EB[e1 + 1]);
                match k {
                    Const(b) => add(FULL, FULL, c, e, w, b),
                    River => {
                        add(TB[0], FULL, c, e, w, "frozen_river");
                        add((-4500, 10000), FULL, c, e, w, "river");
                    }
                    SwampValley => {
                        add(TB[0], FULL, c, e, w, "frozen_river");
                        add((-4500, 2000), FULL, c, e, w, "swamp");
                        add((2000, 10000), FULL, c, e, w, "mangrove_swamp");
                    }
                    Swamp => {
                        for h in 0..5 {
                            add(TB[0], HB[h], c, e, w, middle(0, h, neg));
                        }
                        add((-4500, 2000), FULL, c, e, w, "swamp");
                        add((2000, 10000), FULL, c, e, w, "mangrove_swamp");
                    }
                    _ => {
                        for t in 0..5 {
                            for h in 0..5 {
                                add(TB[t], HB[h], c, e, w, pick(k, t, h, neg));
                            }
                        }
                    }
                }
            }
        }
        v.push(Entry { r: [FULL, FULL, (8000, 10000), FULL, (2000, 9000), FULL, (0, 0)], biome: "dripstone_caves" });
        v.push(Entry { r: [FULL, (7000, 10000), FULL, FULL, (2000, 9000), FULL, (0, 0)], biome: "lush_caves" });
        v.push(Entry { r: [FULL, FULL, FULL, (-10000, -3750), (11000, 11000), FULL, (0, 0)], biome: "deep_dark" });
        BiomeSource { entries: v }
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// Quần xã có tham số gần nhất với điểm khí hậu (nhiệt độ, độ ẩm, lục địa, xói mòn, độ sâu, độ kỳ quặc).
    pub fn find(&self, t: &[i64; 6]) -> &'static str {
        let mut best = i64::MAX;
        let mut out = "plains";
        for e in &self.entries {
            let mut s = 0i64;
            for i in 0..6 {
                let (lo, hi) = e.r[i];
                let d = if t[i] > hi {
                    t[i] - hi
                } else if lo > t[i] {
                    lo - t[i]
                } else {
                    0
                };
                s += d * d;
                if s >= best {
                    break;
                }
            }
            if s < best {
                best = s;
                out = e.biome;
            }
        }
        out
    }
}

impl BiomeSource {
    /// Mọi tên quần xã có trong bảng (không trùng).
    pub fn names(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Vec::new();
        for e in &self.entries {
            if !v.contains(&e.biome) {
                v.push(e.biome);
            }
        }
        v
    }
}
