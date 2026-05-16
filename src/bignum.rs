//! `Mag` — a non-negative real represented as `log10(value)`.
//!
//! The whole point: gameplay-side quantities (cuques, FPS, tree multipliers,
//! per-fingerer modifier mults) compound multiplicatively across thousands
//! of bought nodes and powerup catches. Stored as `f64`, the product
//! reaches `Infinity` after a long-haul session — the headline bug from
//! the wild "broken save" triage. Stored as `log10`, the same product
//! fits comfortably inside an `f64` for any value the player could
//! conceivably observe; the dynamic range is `10^(±1.8e308)`, i.e.
//! "truly infinite" for any practical gameplay.
//!
//! ## Representation
//!
//! `Mag { log10: f64 }`, where `log10` is the base-10 logarithm of the
//! value. Zero is the sentinel `log10 == f64::NEG_INFINITY`; everything
//! else is finite. Negatives are not representable (the only place the
//! game produced one was an underflow that signalled a bug anyway —
//! `try_sub` returns `Option`, callers handle the deficit explicitly).
//!
//! ## Operations
//!
//! - `mul` / `div`: addition / subtraction in log space — cheap.
//! - `add`: requires un-logging the smaller addend; uses
//!   `log10(a + b) = log10(a) + log10(1 + 10^(b - a))` for `a >= b`.
//! - `try_sub`: same trick. Returns `None` if the result would be
//!   negative.
//! - `cmp`: direct comparison of the log values.
//! - `to_f64`: useful at display sites (for the legacy `format::big`
//!   path) and inside formulas that need a real `f64` (e.g. RNG range
//!   bounds, multiplier caps from the procgen).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Non-negative log-magnitude. `Mag::ZERO == Mag { log10: -inf }`,
/// `Mag::ONE == Mag { log10: 0.0 }`. All other instances carry a
/// finite `log10`.
///
/// ## Save compatibility
///
/// Serializes as a plain `f64` when the value fits inside one
/// (`log10 < 300`, well clear of overflow); otherwise as
/// `{"log10": <f64>}`. Deserialization accepts BOTH shapes — so existing
/// V4 saves with `"cuques": 1.234e10` and `"MulFactor": 7.0` continue
/// to load correctly, no schema bump required. New saves use the
/// struct form only when the value exceeds the `f64` finite range.
#[derive(Clone, Copy, Debug)]
pub struct Mag {
    pub log10: f64,
}

impl Serialize for Mag {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Below the `f64` overflow horizon, emit a plain number so saves
        // stay human-readable and old-format-compatible. Above it, fall
        // back to the explicit struct so the log-magnitude survives the
        // round-trip exactly.
        if self.log10 == f64::NEG_INFINITY {
            return 0.0_f64.serialize(s);
        }
        if self.log10 < 300.0 {
            return self.to_f64().serialize(s);
        }
        // Struct form for truly-huge values.
        #[derive(Serialize)]
        struct Wide {
            log10: f64,
        }
        Wide { log10: self.log10 }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Mag {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Accept either a plain JSON number (legacy + small new saves)
        // or the explicit `{"log10": ...}` struct (large new saves). The
        // untagged enum gives serde the freedom to try both without us
        // hand-writing a visitor.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Number(f64),
            Struct { log10: f64 },
        }
        match Either::deserialize(d)? {
            Either::Number(v) => Ok(Mag::from_f64(v)),
            Either::Struct { log10 } => Ok(Mag { log10 }),
        }
    }
}

impl Mag {
    pub const ZERO: Mag = Mag {
        log10: f64::NEG_INFINITY,
    };
    pub const ONE: Mag = Mag { log10: 0.0 };

    /// Construct from an `f64`. Non-finite / negative inputs collapse to
    /// `ZERO` — there is no honest log-magnitude for them and silently
    /// turning a `NaN` into a zero is friendlier than panicking on a
    /// codepath the player never asked for.
    pub fn from_f64(v: f64) -> Mag {
        if !v.is_finite() || v <= 0.0 {
            Mag::ZERO
        } else {
            Mag { log10: v.log10() }
        }
    }

    /// Decode back to an `f64`. Clamps very-large `log10` values to
    /// `f64::MAX` — losing precision in the high end is the price of
    /// asking for an `f64`; the `Mag` representation itself stays
    /// faithful. Use this at the display boundary or wherever a downstream
    /// API truly demands `f64`. Internal math should chain `Mag` ops.
    pub fn to_f64(self) -> f64 {
        if self.log10.is_nan() {
            return 0.0;
        }
        if self.log10 == f64::NEG_INFINITY {
            return 0.0;
        }
        if self.log10 >= f64::MAX.log10() {
            return f64::MAX;
        }
        10f64.powf(self.log10)
    }

    pub fn is_zero(self) -> bool {
        self.log10 == f64::NEG_INFINITY
    }

    // `clippy::should_implement_trait` would have us drop the inherent
    // method in favor of the `std::ops::Mul` impl below. We keep both
    // because the call sites read more naturally as `a.mul(b)` than
    // `Mag::Mul::mul(a, b)` and the `*` operator alone obscures that
    // Mag isn't a primitive `f64`. The trait impl exists so `*` works
    // and so the inherent method is provably aligned with std semantics.
    #[allow(clippy::should_implement_trait)]
    pub fn mul(self, rhs: Mag) -> Mag {
        if self.is_zero() || rhs.is_zero() {
            return Mag::ZERO;
        }
        Mag {
            log10: self.log10 + rhs.log10,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn div(self, rhs: Mag) -> Mag {
        if self.is_zero() {
            return Mag::ZERO;
        }
        if rhs.is_zero() {
            // Defining 1/0 as ZERO here is a deliberate, defensive
            // choice: nothing in gameplay should ever divide by zero,
            // but if it does we'd rather collapse to "no contribution"
            // than panic or produce an `Infinity` that re-introduces
            // the bug class this whole module exists to kill.
            return Mag::ZERO;
        }
        Mag {
            log10: self.log10 - rhs.log10,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn add(self, rhs: Mag) -> Mag {
        if self.is_zero() {
            return rhs;
        }
        if rhs.is_zero() {
            return self;
        }
        let (hi, lo) = if self.log10 >= rhs.log10 {
            (self.log10, rhs.log10)
        } else {
            (rhs.log10, self.log10)
        };
        let diff = lo - hi;
        if diff < -16.0 {
            // 10^-16 is past f64 precision relative to 1.0 — the smaller
            // addend's contribution rounds away. Skip the (1.0 + 10^diff)
            // dance and return the larger.
            return Mag { log10: hi };
        }
        Mag {
            log10: hi + (1.0 + 10f64.powf(diff)).log10(),
        }
    }

    /// Saturating subtraction: returns `Some(self - rhs)` when
    /// `self >= rhs`, otherwise `None`. Cuque spending uses this on
    /// purchase — the caller already gated on `affordable`, so `None`
    /// means a logic bug, not a regular game state.
    pub fn try_sub(self, rhs: Mag) -> Option<Mag> {
        if rhs.is_zero() {
            return Some(self);
        }
        if self.log10 < rhs.log10 {
            return None;
        }
        if self.is_zero() {
            return Some(Mag::ZERO);
        }
        let diff = rhs.log10 - self.log10;
        if diff < -16.0 {
            return Some(self);
        }
        let factor = 1.0 - 10f64.powf(diff);
        if factor <= 0.0 {
            return Some(Mag::ZERO);
        }
        Some(Mag {
            log10: self.log10 + factor.log10(),
        })
    }

    /// Saturating subtraction with floor at zero. Convenience for paths
    /// that don't care about the underflow case (e.g. visual tween
    /// targets).
    pub fn saturating_sub(self, rhs: Mag) -> Mag {
        self.try_sub(rhs).unwrap_or(Mag::ZERO)
    }

    /// Raise to an integer power. `pow(0) == ONE`, `pow(n) = self * self * …`.
    /// Cheap: just multiplies the log by `n`.
    ///
    /// `0^n` for any `n != 0` returns `ZERO`. Mathematically `0^negative` is
    /// undefined, but returning ZERO is the conservative choice for this
    /// codebase: it propagates "no contribution" downstream rather than
    /// producing an `Infinity`, which would reintroduce the bug class
    /// the `Mag` type exists to kill.
    pub fn pow_i(self, n: i32) -> Mag {
        if n == 0 {
            return Mag::ONE;
        }
        if self.is_zero() {
            return Mag::ZERO;
        }
        Mag {
            log10: self.log10 * (n as f64),
        }
    }

    /// Floored `to_f64`, useful for "how many of these can the player
    /// afford" math. Clamps to `u64::MAX`.
    pub fn floor_u64(self) -> u64 {
        let v = self.to_f64().floor();
        if v <= 0.0 {
            0
        } else if v >= u64::MAX as f64 {
            u64::MAX
        } else {
            v as u64
        }
    }
}

impl Default for Mag {
    fn default() -> Self {
        Mag::ZERO
    }
}

impl PartialEq for Mag {
    fn eq(&self, other: &Self) -> bool {
        // NaN-safe: treat any NaN log as ZERO for equality purposes.
        let a = if self.log10.is_nan() {
            f64::NEG_INFINITY
        } else {
            self.log10
        };
        let b = if other.log10.is_nan() {
            f64::NEG_INFINITY
        } else {
            other.log10
        };
        a == b
    }
}

impl PartialOrd for Mag {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.log10.partial_cmp(&other.log10)
    }
}

// Convenience: ergonomic conversion from common literal types so call
// sites can keep reading naturally ("Mag::from(0.05)" inside a test
// fixture).
impl From<f64> for Mag {
    fn from(v: f64) -> Self {
        Mag::from_f64(v)
    }
}

impl From<u32> for Mag {
    fn from(v: u32) -> Self {
        Mag::from_f64(v as f64)
    }
}

impl From<u64> for Mag {
    fn from(v: u64) -> Self {
        Mag::from_f64(v as f64)
    }
}

// Operator traits forward to the inherent methods so `a * b` / `a + b` /
// `a / b` work alongside the explicit `.mul(b)` / `.add(b)` / `.div(b)`
// the rest of the codebase uses. The trait impls also silence clippy's
// `method_can_be_confused_for_std_trait` warning — the methods *are*
// the standard operations now.
//
// Subtraction deliberately doesn't get a `Sub` impl: the only honest
// answer for `a - b` when `b > a` is "underflow", and `try_sub →
// Option` keeps every spend site forced to acknowledge that case.

impl std::ops::Mul for Mag {
    type Output = Mag;
    fn mul(self, rhs: Mag) -> Mag {
        Mag::mul(self, rhs)
    }
}

impl std::ops::Div for Mag {
    type Output = Mag;
    fn div(self, rhs: Mag) -> Mag {
        Mag::div(self, rhs)
    }
}

impl std::ops::Add for Mag {
    type Output = Mag;
    fn add(self, rhs: Mag) -> Mag {
        Mag::add(self, rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64) -> bool {
        if a == b {
            return true;
        }
        (a - b).abs() / b.abs().max(1e-12) < 1e-10
    }

    #[test]
    fn from_f64_roundtrip() {
        for v in [1.0, 1000.0, 1e20, 1e100, 1e300] {
            let m = Mag::from_f64(v);
            assert!(approx_eq(m.to_f64(), v), "roundtrip {v} got {}", m.to_f64());
        }
    }

    #[test]
    fn zero_and_negative_collapse_to_zero() {
        assert_eq!(Mag::from_f64(0.0), Mag::ZERO);
        assert_eq!(Mag::from_f64(-5.0), Mag::ZERO);
        assert_eq!(Mag::from_f64(f64::NAN), Mag::ZERO);
        assert_eq!(Mag::from_f64(f64::INFINITY), Mag::ZERO);
    }

    #[test]
    fn mul_compounds_huge() {
        let a = Mag::from_f64(1e200);
        let b = Mag::from_f64(1e200);
        let c = a.mul(b);
        // 10^400 — well past f64::MAX.
        assert!(approx_eq(c.log10, 400.0));
        // to_f64 saturates rather than producing infinity.
        assert_eq!(c.to_f64(), f64::MAX);
    }

    #[test]
    fn pow_i_handles_grindy_compounding() {
        // 1.005^100_000 — a stress test for late-late-game stacks.
        // Original f64 path would overflow long before this.
        let m = Mag::from_f64(1.005);
        let p = m.pow_i(100_000);
        let expected_log = (1.005_f64).log10() * 100_000.0;
        assert!(approx_eq(p.log10, expected_log));
    }

    #[test]
    fn add_handles_disparate_magnitudes() {
        // 10^100 + 10^50 ≈ 10^100 (within f64 precision).
        let a = Mag::from_f64(1e100);
        let b = Mag::from_f64(1e50);
        let s = a.add(b);
        assert!(approx_eq(s.log10, a.log10));
    }

    #[test]
    fn add_handles_equal_magnitudes() {
        let a = Mag::from_f64(1e100);
        let s = a.add(a);
        // 2 * 10^100 -> log10 = 100 + log10(2)
        assert!(approx_eq(s.log10, 100.0 + 2.0_f64.log10()));
    }

    #[test]
    fn try_sub_returns_none_on_underflow() {
        let a = Mag::from_f64(100.0);
        let b = Mag::from_f64(200.0);
        assert!(a.try_sub(b).is_none());
    }

    #[test]
    fn try_sub_handles_close_values() {
        let a = Mag::from_f64(100.0);
        let b = Mag::from_f64(99.0);
        let s = a.try_sub(b).expect("should fit");
        assert!(approx_eq(s.to_f64(), 1.0));
    }

    #[test]
    fn try_sub_zero_is_identity() {
        let a = Mag::from_f64(50.0);
        assert_eq!(a.try_sub(Mag::ZERO).unwrap(), a);
    }

    #[test]
    fn comparison_order_matches_value_order() {
        assert!(Mag::from_f64(10.0) < Mag::from_f64(100.0));
        assert!(Mag::ZERO < Mag::from_f64(1.0));
        // Truly huge values still order correctly.
        assert!(Mag::from_f64(1e200).mul(Mag::from_f64(1e200)) > Mag::from_f64(1e300));
    }

    #[test]
    fn mul_by_zero_is_zero() {
        let a = Mag::from_f64(1e100);
        assert_eq!(a.mul(Mag::ZERO), Mag::ZERO);
    }

    #[test]
    fn one_is_multiplicative_identity() {
        let a = Mag::from_f64(42.0);
        assert!(approx_eq(a.mul(Mag::ONE).to_f64(), a.to_f64()));
        assert!(approx_eq(Mag::ONE.mul(a).to_f64(), a.to_f64()));
    }

    #[test]
    fn floor_u64_clamps_at_u64_max() {
        let huge = Mag::from_f64(1e25);
        // 10^25 > u64::MAX (~1.8e19), should clamp.
        assert_eq!(huge.floor_u64(), u64::MAX);
        let small = Mag::from_f64(42.7);
        assert_eq!(small.floor_u64(), 42);
        assert_eq!(Mag::ZERO.floor_u64(), 0);
    }
}
