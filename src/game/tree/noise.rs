//! Deterministic value noise — hash-smoothed lattice sampling.
//!
//! We don't need Perlin's gradient noise here; value noise is fine for
//! biome regions and rarity hot-spots. It's a pure function of (seed, x, y)
//! so the tree is byte-identical across players and platforms.

use super::rng::mix_u64;

/// Hash a lattice point `(ix, iy)` into `[0.0, 1.0)`. Uses the SplitMix64
/// avalanche over a tightly-packed coord encoding so adjacent lattice
/// values don't correlate.
fn hash_lattice(seed: u64, ix: i32, iy: i32) -> f64 {
    let packed = seed
        ^ ((ix as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        ^ ((iy as i64 as u64)
            .wrapping_mul(0xBF58_476D_1CE4_E5B9)
            .rotate_left(17));
    let h = mix_u64(packed);
    (h >> 11) as f64 / ((1u64 << 53) as f64)
}

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// 2D value noise sampled at `(x, y)`. Returns a value in `[0.0, 1.0)`
/// that's smooth (C1-ish, smoothstep-interpolated) between integer lattice
/// points.
pub fn value_noise_2d(seed: u64, x: f64, y: f64) -> f64 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let xf = x - x.floor();
    let yf = y - y.floor();
    let v00 = hash_lattice(seed, xi, yi);
    let v10 = hash_lattice(seed, xi + 1, yi);
    let v01 = hash_lattice(seed, xi, yi + 1);
    let v11 = hash_lattice(seed, xi + 1, yi + 1);
    let u = smoothstep(xf);
    let v = smoothstep(yf);
    let bottom = lerp(v00, v10, u);
    let top = lerp(v01, v11, u);
    lerp(bottom, top, v)
}

/// Fractal value noise: stacks 3 octaves at halving amplitude and doubling
/// frequency. Gives more interesting biome shapes than a single octave at
/// the cost of three lattice lookups instead of one.
pub fn fbm_2d(seed: u64, x: f64, y: f64) -> f64 {
    let mut total = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut max = 0.0;
    for octave in 0..3 {
        // Salt each octave so its lattice doesn't trivially align with the
        // others (otherwise the octaves stack into a banded artifact).
        let s = seed ^ ((octave as u64).wrapping_mul(0xABCD_EF01_2345_6789));
        total += amp * value_noise_2d(s, x * freq, y * freq);
        max += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    total / max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_noise_is_deterministic() {
        let a = value_noise_2d(123, 1.5, -3.2);
        let b = value_noise_2d(123, 1.5, -3.2);
        assert_eq!(a, b);
    }

    #[test]
    fn value_noise_stays_in_unit_range() {
        for x in -50..50 {
            for y in -50..50 {
                let v = value_noise_2d(7, x as f64 * 0.31, y as f64 * 0.17);
                assert!((0.0..1.0).contains(&v), "out of range at ({x},{y}): {v}");
            }
        }
    }

    #[test]
    fn value_noise_changes_with_coord() {
        // Adjacent lattice points should hash to distinct values most of the
        // time — assert at least one neighbor differs from the center.
        let center = value_noise_2d(0, 0.0, 0.0);
        let near = value_noise_2d(0, 5.0, 5.0);
        assert_ne!(center, near);
    }

    #[test]
    fn fbm_stays_in_unit_range() {
        for x in -20..20 {
            for y in -20..20 {
                let v = fbm_2d(99, x as f64 * 0.7, y as f64 * 0.7);
                assert!((0.0..=1.0).contains(&v), "out of range: {v}");
            }
        }
    }
}
