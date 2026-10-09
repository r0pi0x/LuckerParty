//! Randomness client and server agree on (specs/cs_source/weapons.md 4.2):
//! every roll on the predicted path (shot spread, recoil kicks) comes from
//! the command number, so re-running a command gives the same result and a
//! server recomputes it rather than trusting the client.
//!
//! - A command's random seed is `MD5(n as 4 LE bytes)`, the 32-bit LE
//!   integer at digest bytes 6-9, `& 0x7FFFFFFF` (`command_seed`).
//! - Shots use `seed & 255` (`shot_seed`); pellet `i` reseeds the
//!   generator with `shot seed + 1 + i` (the template's rule).
//! - Other shared rolls seed it with `CRC32(S as i32 LE ‖ extra as i32 LE ‖
//!   label)` (`shared_random`).
//!
//! The generator itself is ours (`Rng`): CS:S's is not in the SDK (spec
//! Q1), so its exact patterns aren't reproduced.

/// The command's random seed `S` (spec 4.2, test T1).
pub fn command_seed(command: u32) -> u32 {
    let d = md5_4(command.to_le_bytes());
    u32::from_le_bytes([d[6], d[7], d[8], d[9]]) & 0x7FFF_FFFF
}

/// The seed shots of command `command` use (256 patterns).
pub fn shot_seed(command: u32) -> u32 {
    command_seed(command) & 255
}

/// A generator for one shared roll of command `command`: `label` names
/// the use (a fixed text per kind of roll), `extra` tells several rolls of
/// one use apart.
pub fn shared_random(command: u32, label: &str, extra: i32) -> Rng {
    let mut bytes = Vec::with_capacity(8 + label.len());
    bytes.extend_from_slice(&(command_seed(command) as i32).to_le_bytes());
    bytes.extend_from_slice(&extra.to_le_bytes());
    bytes.extend_from_slice(label.as_bytes());
    Rng::new(crc32(&bytes))
}

/// Small deterministic generator (splitmix64), uniform in [0, 1).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed as u64 ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// CRC-32 (IEEE 802.3, reflected, as zlib's).
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// MD5 (RFC 1321) of a 4-byte message: one padded block.
fn md5_4(msg: [u8; 4]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14,
        20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6,
        10, 15, 21,
    ];
    // floor(|sin(i + 1)| × 2^32), written out (no libm in the result).
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8,
        0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340,
        0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87,
        0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
        0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039,
        0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92,
        0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut block = [0u8; 64];
    block[..4].copy_from_slice(&msg);
    block[4] = 0x80;
    block[56..64].copy_from_slice(&(32u64).to_le_bytes());
    let m: Vec<u32> = block
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let (a0, b0, c0, d0) = (0x6745_2301u32, 0xEFCD_AB89u32, 0x98BA_DCFEu32, 0x1032_5476u32);
    let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(f.rotate_left(S[i]));
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0.wrapping_add(a), b0.wrapping_add(b), c0.wrapping_add(c), d0.wrapping_add(d)]
        .iter()
        .enumerate()
    {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_the_rfc_example() {
        // A 4-byte message's well-known digest: MD5("abcd").
        let d = md5_4(*b"abcd");
        let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "e2fc714c4727ee9395f324cd2e7f331f");
    }

    #[test]
    fn command_seeds_match_spec_t1() {
        // weapons.md tests T1 and T1b.
        assert_eq!(command_seed(1), 1_997_239_609);
        assert_eq!(shot_seed(1), 57);
        for (n, s, shot) in [
            (2, 1_318_994_272, 96),
            (100, 236_147_958, 246),
            (12345, 36_020_103, 135),
        ] {
            assert_eq!(command_seed(n), s, "command {n}");
            assert_eq!(shot_seed(n), shot, "command {n}");
        }
    }

    #[test]
    fn crc32_matches_the_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn shared_rolls_differ_by_label_and_extra() {
        let roll = |label, extra| shared_random(7, label, extra).next();
        assert_eq!(roll("a", 0), roll("a", 0));
        assert_ne!(roll("a", 0), roll("b", 0));
        assert_ne!(roll("a", 0), roll("a", 1));
    }

    #[test]
    fn rng_is_uniform_enough() {
        let mut r = Rng::new(7);
        let n = 10_000;
        let mean: f32 = (0..n).map(|_| r.next()).sum::<f32>() / n as f32;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
    }
}
