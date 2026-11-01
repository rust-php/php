//! Port of ext/random/engine_mt19937.c and the range algorithm of random.c / mt_rand.c.
use std::cell::RefCell;

const N: usize = 624;
const M: usize = 397;

pub struct Mt {
    state: [u32; N],
    count: usize,
    pub mode_php: bool,
    seeded: bool,
}

thread_local! {
    static MT: RefCell<Mt> = RefCell::new(Mt { state: [0; N], count: N, mode_php: false, seeded: false });
}

fn twist(m: u32, u: u32, v: u32) -> u32 {
    let mix = (u & 0x8000_0000) | (v & 0x7FFF_FFFF);
    m ^ (mix >> 1) ^ ((0u32.wrapping_sub(v & 1)) & 0x9908_b0df)
}
fn twist_php(m: u32, u: u32, v: u32) -> u32 {
    let mix = (u & 0x8000_0000) | (v & 0x7FFF_FFFF);
    m ^ (mix >> 1) ^ ((0u32.wrapping_sub(u & 1)) & 0x9908_b0df)
}

impl Mt {
    fn reload(&mut self) {
        let tw: fn(u32, u32, u32) -> u32 = if self.mode_php { twist_php } else { twist };
        let s = &mut self.state;
        let mut p = 0usize;
        for _ in 0..(N - M) {
            s[p] = tw(s[p + M], s[p], s[p + 1]);
            p += 1;
        }
        for _ in 1..M {
            s[p] = tw(s[p + M - N], s[p], s[p + 1]);
            p += 1;
        }
        s[p] = tw(s[p + M - N], s[p], s[0]);
        self.count = 0;
    }

    pub fn seed32(&mut self, seed: u32) {
        self.state[0] = seed;
        for i in 1..N {
            let prev = self.state[i - 1];
            self.state[i] = 1812433253u32.wrapping_mul(prev ^ (prev >> 30)).wrapping_add(i as u32);
        }
        self.count = N;
        self.reload();
        self.seeded = true;
    }

    pub fn generate(&mut self) -> u32 {
        if self.count >= N {
            self.reload();
        }
        let mut s1 = self.state[self.count];
        self.count += 1;
        s1 ^= s1 >> 11;
        s1 ^= (s1 << 7) & 0x9d2c_5680;
        s1 ^= (s1 << 15) & 0xefc6_0000;
        s1 ^ (s1 >> 18)
    }

    fn range32(&mut self, umax: u32) -> u32 {
        let mut result = self.generate();
        if umax == u32::MAX {
            return result;
        }
        let umax = umax + 1;
        if umax & (umax - 1) == 0 {
            return result & (umax - 1);
        }
        let limit = u32::MAX - (u32::MAX % umax) - 1;
        while result > limit {
            result = self.generate();
        }
        result % umax
    }

    fn range64(&mut self, umax: u64) -> u64 {
        let gen64 = |s: &mut Mt| {
            let lo = s.generate() as u64;
            let hi = s.generate() as u64;
            lo | (hi << 32)
        };
        let mut result = gen64(self);
        if umax == u64::MAX {
            return result;
        }
        let umax = umax + 1;
        if umax & (umax - 1) == 0 {
            return result & (umax - 1);
        }
        let limit = u64::MAX - (u64::MAX % umax) - 1;
        while result > limit {
            result = gen64(self);
        }
        result % umax
    }

    pub fn range(&mut self, min: i64, max: i64) -> i64 {
        let umax = (max as u64).wrapping_sub(min as u64);
        if umax > u32::MAX as u64 {
            return self.range64(umax).wrapping_add(min as u64) as i64;
        }
        (self.range32(umax as u32) as u64).wrapping_add(min as u64) as i64
    }
}

fn entropy_seed() -> u32 {
    let mut b = [0u8; 4];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        if f.read_exact(&mut b).is_ok() {
            return u32::from_le_bytes(b);
        }
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u32).unwrap_or(1)
}

pub fn with<T>(f: impl FnOnce(&mut Mt) -> T) -> T {
    MT.with(|m| {
        let mut m = m.borrow_mut();
        if !m.seeded {
            m.mode_php = false;
            m.seed32(entropy_seed());
        }
        f(&mut m)
    })
}

pub fn seed(seed: Option<i64>, php_mode: bool) {
    MT.with(|m| {
        let mut m = m.borrow_mut();
        m.mode_php = php_mode;
        m.seed32(match seed {
            Some(s) => s as u32,
            None => entropy_seed(),
        });
    })
}

pub fn mt_rand_range(min: i64, max: i64) -> i64 {
    with(|m| m.range(min, max))
}

/// php_mt_rand_common
pub fn mt_rand_common(min: i64, max: i64) -> i64 {
    with(|m| {
        if !m.mode_php {
            return m.range(min, max);
        }
        let r = (m.generate() >> 1) as u64;
        let offset = ((max as f64 - min as f64 + 1.0) * (r as f64 / (2147483647u32 as f64 + 1.0))) as u64;
        offset.wrapping_add(min as u64) as i64
    })
}

pub fn mt_rand_31() -> i64 {
    with(|m| (m.generate() >> 1) as i64)
}

pub fn secure_u64() -> u64 {
    let mut b = [0u8; 8];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut b);
    }
    u64::from_le_bytes(b)
}

/// CSPRNG range (random_int).
pub fn secure_range(min: i64, max: i64) -> i64 {
    let umax = (max as u64).wrapping_sub(min as u64);
    if umax == u64::MAX {
        return secure_u64() as i64;
    }
    let um = umax + 1;
    let mut r = secure_u64();
    if um & (um - 1) == 0 {
        return (r & (um - 1)).wrapping_add(min as u64) as i64;
    }
    let limit = u64::MAX - (u64::MAX % um) - 1;
    while r > limit {
        r = secure_u64();
    }
    (r % um).wrapping_add(min as u64) as i64
}
