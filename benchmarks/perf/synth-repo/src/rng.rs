//! Deterministic RNG: SplitMix64 seeded from `blake3(seed || tag || index)`, so every module is
//! generated independently of every other and of the thread count.

#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn seeded(seed: u64, tag: &str, index: u64) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&seed.to_le_bytes());
        hasher.update(tag.as_bytes());
        hasher.update(&index.to_le_bytes());
        let out = hasher.finalize();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&out.as_bytes()[..8]);
        Self(u64::from_le_bytes(bytes))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n == 0` yields 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi.saturating_sub(lo) + 1)
    }
}

/// Cheap integer mix used for names, where no stream state is wanted.
pub fn mix(seed: u64, a: u64, b: u64) -> u64 {
    let mut rng = Rng(seed ^ a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.rotate_left(32));
    rng.next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_same_stream() {
        let mut a = Rng::seeded(1, "m", 2);
        let mut b = Rng::seeded(1, "m", 2);
        assert_eq!(a.next_u64(), b.next_u64());
        assert_ne!(
            Rng::seeded(1, "m", 3).next_u64(),
            Rng::seeded(1, "m", 2).next_u64()
        );
    }

    #[test]
    fn range_is_inclusive_and_bounded() {
        let mut rng = Rng::seeded(9, "r", 0);
        for _ in 0..1000 {
            let v = rng.range(3, 5);
            assert!((3..=5).contains(&v));
        }
    }
}
