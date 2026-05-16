//! The one true seed.
//!
//! The literal string lives here so it's grep-able forever. We hash it with
//! a `const fn` FNV-1a 64-bit so the resulting `u64` is computed at compile
//! time and never changes between builds or platforms. **Do not change this
//! string.** Doing so would silently shift every player's tree.

pub const TREE_SEED_LITERAL: &str = "DEADBEEF_BANANA";
pub const TREE_SEED: u64 = fnv1a_64(TREE_SEED_LITERAL.as_bytes());

/// FNV-1a 64-bit hash. Stable across Rust versions and platforms because it's
/// a pure const-time integer operation — no external crates, no platform
/// hashing primitive.
pub const fn fnv1a_64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_deterministic_and_nonzero() {
        // If this ever changes, every existing player's tree shifts — it's
        // load-bearing. The exact value below is the FNV-1a 64 of
        // "DEADBEEF_BANANA". Pinning it as a literal here makes accidental
        // drift impossible without re-baking the test fixture.
        assert_eq!(TREE_SEED, fnv1a_64(b"DEADBEEF_BANANA"));
        assert_ne!(TREE_SEED, 0);
    }

    #[test]
    fn fnv1a_matches_known_vector() {
        // FNV-1a 64 of the empty string is the offset basis.
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        // Known-good vector: "a" → 0xaf63dc4c8601ec8c
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        // "foobar" → 0x85944171f73967e8
        assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
    }
}
