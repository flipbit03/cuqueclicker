//! Position-seeded deterministic RNG for tree generation.
//!
//! SplitMix64 — small state, fast, statistically fine for procgen. Every
//! call site seeds a fresh instance from `(TREE_SEED, lot_x, lot_y, salt)`
//! and consumes it locally, so generation is referentially transparent: two
//! calls to `node_at(3, -4)` produce byte-identical results.

/// Tiny deterministic PRNG. State is one u64; output passes the obvious
/// goodness tests (BigCrush-ish) and is more than enough for upgrade-tree
/// procgen. Not for cryptography.
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Combine a base seed with three coordinate-like ints into a fresh
    /// PRNG state. `salt` lets adjacent call sites avoid stepping on each
    /// other (e.g. "noise lookup" vs "edge roll" at the same lot).
    pub fn from_coords(seed: u64, x: i32, y: i32, salt: u64) -> Self {
        // Mix the coords through SplitMix64's avalanche step before XORing
        // into the base seed so neighboring (x, y) pairs don't produce
        // trivially-correlated states.
        let mix = mix_u64(
            seed ^ salt
                ^ ((x as i64 as u64).rotate_left(13))
                ^ ((y as i64 as u64)
                    .rotate_left(31)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        );
        Self::new(mix)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix_u64(self.state)
    }

    /// Returns a value in `[0.0, 1.0)`. 53-bit mantissa for double precision.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    /// Returns a value in `[lo, hi)`.
    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    /// Returns an integer in `[0, max)`. Trivial modulo bias is acceptable
    /// for procgen — the bias for `max < 1e9` is well below any other source
    /// of variance in the system.
    pub fn range_usize(&mut self, max: usize) -> usize {
        if max == 0 {
            return 0;
        }
        (self.next_u64() % (max as u64)) as usize
    }

    /// True with probability `p`.
    pub fn bool_with_prob(&mut self, p: f64) -> bool {
        self.next_f64() < p
    }
}

/// SplitMix64's avalanche finalizer. Standalone so other modules can fold
/// coords into seeds without spinning up an RNG instance.
pub const fn mix_u64(seed: u64) -> u64 {
    let mut z = seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_coords_different_states() {
        let a = SplitMix64::from_coords(7, 1, 1, 0).state;
        let b = SplitMix64::from_coords(7, 1, 2, 0).state;
        assert_ne!(a, b);
    }

    #[test]
    fn salt_separates_call_sites() {
        let a = SplitMix64::from_coords(7, 1, 1, 0).state;
        let b = SplitMix64::from_coords(7, 1, 1, 1).state;
        assert_ne!(a, b);
    }

    #[test]
    fn range_f64_within_bounds() {
        let mut r = SplitMix64::new(1234);
        for _ in 0..1000 {
            let v = r.range_f64(-2.0, 5.0);
            assert!(v >= -2.0 && v < 5.0, "{}", v);
        }
    }

    #[test]
    fn range_usize_within_bounds() {
        let mut r = SplitMix64::new(1234);
        for _ in 0..1000 {
            let v = r.range_usize(10);
            assert!(v < 10);
        }
    }
}
