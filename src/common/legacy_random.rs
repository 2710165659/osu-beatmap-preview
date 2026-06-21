//! osu! FastRandom（xorshift128）与无状态 MurmurHash3-finalizer 随机数。
//! 调用顺序必须与 lazer / stable 的实现完全一致（决定香蕉/水滴的随机位置）。

pub struct LegacyRandom {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl LegacyRandom {
    pub fn new(seed: u32) -> Self {
        LegacyRandom {
            x: seed,
            y: 842502087,
            z: 3579807591,
            w: 273326509,
        }
    }

    /// xorshift128 核心步进。
    pub fn next_uint(&mut self) -> u32 {
        let t = self.x ^ (self.x << 11);
        self.x = self.y;
        self.y = self.z;
        self.z = self.w;
        self.w = self.w ^ (self.w >> 19) ^ t ^ (t >> 8);
        self.w
    }

    /// 非负 31 位整数（与 .NET Random.Next() 语义对应）。
    pub fn next(&mut self) -> i32 {
        (self.next_uint() & 0x7FFF_FFFF) as i32
    }

    /// [lower, upper) 区间整数。
    pub fn next_range(&mut self, lower: i32, upper: i32) -> i32 {
        (lower as f64 + self.next_double() * (upper - lower) as f64) as i32
    }

    /// [0, 1) 双精度。
    pub fn next_double(&mut self) -> f64 {
        (self.next_uint() & 0x7FFF_FFFF) as f64 / 2147483648.0
    }
}

/// MurmurHash3 finalizer 混淆。
fn stateless_mix(mut value: u64) -> u64 {
    value ^= value >> 33;
    value = value.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    value ^= value >> 33;
    value = value.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    value ^= value >> 33;
    value
}

/// 无状态随机：由（seed, series）直接得到 u64（lazer StatelessRNG）。
fn stateless_next_ulong(seed: i64, series: i64) -> u64 {
    let combined = (((series as u64) & 0xFFFF_FFFF) << 32) | ((seed as u64) & 0xFFFF_FFFF);
    stateless_mix(combined ^ 0x1234_5678)
}

/// 无状态随机：[0, max_value) 区间整数（用于香蕉颜色选择）。
pub fn stateless_next_int(max_value: u64, seed: i64, series: i64) -> u64 {
    stateless_next_ulong(seed, series) % max_value
}
