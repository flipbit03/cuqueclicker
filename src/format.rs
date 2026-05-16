/// Short suffixes for the first 12 engineering steps (10^0 … 10^33).
/// After Decillion the formatter switches to an *infinite* alphabetic
/// scheme (see `alpha_suffix`).
const KNOWN_SUFFIXES: &[&str] = &[
    "", "k", "M", "B", "T", "Qa", "Qi", "Sx", "Sp", "Oc", "No", "Dc",
];

/// Format a `Mag` (log-magnitude) the same way `big` formats an `f64`,
/// but without an upper bound on representable values. Cuques, FPS,
/// and costs all flow through here once they live in `Mag` form.
pub fn big_mag(m: crate::bignum::Mag) -> String {
    if m.is_zero() {
        return "0".into();
    }
    if m.log10 < 3.0 {
        // Below 1000 — defer to the regular `big()` path for parity
        // with how the game has always rendered small early-game
        // numbers (plain integer, no suffix).
        return big(m.to_f64());
    }
    // Engineering-group: every 3 OOM gets a suffix.
    // `log10 / 3.0` for normal play never exceeds a few thousand, so
    // the as-i64 cast is safe; the upper-bound clamp guards a runaway
    // future bug from indexing past the alpha-suffix horizon.
    let raw_group = (m.log10 / 3.0).floor();
    if !raw_group.is_finite() || raw_group > 1e15 {
        // Past `10^(3e15)` we've left any reasonable gameplay regime —
        // emit a compact log-space label so the HUD still says something
        // useful instead of stalling on an infinite suffix lookup.
        return format!("10^{:.0}", m.log10);
    }
    let group = raw_group as usize;
    let suffix = if group < KNOWN_SUFFIXES.len() {
        KNOWN_SUFFIXES[group].to_string()
    } else {
        alpha_suffix(group - KNOWN_SUFFIXES.len())
    };
    let scaled = 10f64.powf(m.log10 - (group as f64) * 3.0);
    format!("{scaled:.2}{suffix}")
}

pub fn big(n: f64) -> String {
    if n.is_nan() || n.is_infinite() {
        // Bug sentinel: the gameplay math should never reach Inf/NaN
        // with the rebalanced tree magnitudes. If we ever see `?` in
        // game again, something has overflowed and needs investigating.
        return "?".into();
    }
    if n.abs() < 1000.0 {
        return format!("{}", n.floor() as i64);
    }
    let mag = (n.abs().log10() / 3.0).floor() as i32;
    let mag = mag.max(0) as usize;
    let suffix = if mag < KNOWN_SUFFIXES.len() {
        KNOWN_SUFFIXES[mag].to_string()
    } else {
        alpha_suffix(mag - KNOWN_SUFFIXES.len())
    };
    let scaled = n / 10f64.powi((mag * 3) as i32);
    format!("{scaled:.2}{suffix}")
}

/// Infinite alphabetic suffix sequence. `n=0` yields the first suffix
/// past Decillion. The sequence is grouped into *phases*; phase `L≥0`
/// produces all `(L+2)`-letter strings, first all-lowercase, then
/// capital-first. Each phase has `2 × 26^(L+2)` entries.
///
/// Concretely:
///
/// ```text
/// n = 0..676            → aa, ab, …, az, ba, …, zz       (676 = 26²  lowercase pairs)
/// n = 676..1352         → Aa, Ab, …, Az, Ba, …, Zz        (676  uppercase-first pairs)
/// n = 1352..1352+17576  → aaa, aab, …, zzz               (17576 = 26³  lowercase triples)
/// n = …  + 17576        → Aaa, Aab, …, Zzz                (uppercase-first triples)
/// …
/// ```
///
/// Each suffix step represents three more orders of magnitude on top of
/// `10^33 ≈ Dc`, so phase 0 alone covers `10^36 … 10^(33 + 676·3)` =
/// `10^36 … 10^2061` — already past `f64::MAX ≈ 1.8e308`. The higher
/// phases exist so the helper remains correct under any future
/// arbitrary-precision number type.
pub fn alpha_suffix(mut n: usize) -> String {
    let mut len: u32 = 2;
    loop {
        let phase_entries = 26usize.pow(len);
        // Phase L, sub-phase 0: lowercase only.
        if n < phase_entries {
            return digits_to_letters(n, len as usize, false);
        }
        n -= phase_entries;
        // Phase L, sub-phase 1: capital-first.
        if n < phase_entries {
            return digits_to_letters(n, len as usize, true);
        }
        n -= phase_entries;
        len += 1;
    }
}

/// Encode `idx` as a `len`-digit base-26 numeral with leading zeros,
/// emitting `a..z` for each digit. If `cap_first` is true, capitalize
/// the leading letter (e.g. `Aa`, `Aaa`).
fn digits_to_letters(mut idx: usize, len: usize, cap_first: bool) -> String {
    let mut digits = vec![0u8; len];
    for i in (0..len).rev() {
        digits[i] = (idx % 26) as u8;
        idx /= 26;
    }
    digits
        .iter()
        .enumerate()
        .map(|(i, &d)| {
            let base = if i == 0 && cap_first { b'A' } else { b'a' };
            (base + d) as char
        })
        .collect()
}

/// Render a multiplicative magnitude (e.g. a `MulFactor`/`EffectMul` value)
/// with enough decimal places to show ~3 significant figures of its
/// distance from `1.0`. Lets per-tree-node values like `1.00004` actually
/// read as `×1.00004` in the info pane instead of getting rounded to the
/// uninformative `×1.00`.
pub fn mul_magnitude(v: f64) -> String {
    let delta = (v - 1.0).abs();
    if delta == 0.0 || !v.is_finite() {
        return format!("×{v:.2}");
    }
    // Sig-figs of the delta determine precision: `delta = 1e-4` → 6
    // decimals (so the user sees `×1.00010`), `delta = 0.5` → 2 decimals
    // (`×1.50`). Clamped to [2, 8] so we never write e.g. `×1.5` (too
    // few) or `×1.00000123456` (visual noise).
    let need = (-delta.log10()).ceil() as i32 + 2;
    let prec = need.clamp(2, 8) as usize;
    format!("×{v:.*}", prec)
}

/// Like `mul_magnitude`, but for an `AddPercent` magnitude (raw `f64`
/// where `0.01 == 1%`). Picks decimal precision so the user can see
/// sub-percent values like `+0.005%` that would otherwise round to
/// `+0.0%`.
pub fn percent_magnitude(v: f64) -> String {
    let p = v * 100.0;
    let mag = p.abs();
    let prec = if mag >= 1.0 {
        1
    } else if mag >= 0.1 {
        2
    } else if mag >= 0.01 {
        3
    } else {
        4
    };
    format!("{p:+.*}%", prec)
}

/// Like `mul_magnitude`, but for a `FlatAdd` magnitude. Picks decimal
/// precision so a 0.002 FlatAdd doesn't collapse to `+0.0`.
pub fn flat_magnitude(v: f64) -> String {
    let m = v.abs();
    let prec = if m >= 10.0 {
        1
    } else if m >= 1.0 {
        2
    } else if m >= 0.1 {
        3
    } else {
        4
    };
    format!("{v:+.*}", prec)
}

pub fn rate(n: f64) -> String {
    if n < 10.0 {
        format!("{n:.2}")
    } else if n < 100.0 {
        format!("{n:.1}")
    } else {
        big(n)
    }
}

pub fn duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_numbers_are_plain_integers() {
        assert_eq!(big(0.0), "0");
        assert_eq!(big(42.7), "42");
        assert_eq!(big(999.0), "999");
    }

    #[test]
    fn known_suffix_range_uses_short_names() {
        assert_eq!(big(1_500.0), "1.50k");
        assert_eq!(big(2.5e6), "2.50M");
        assert_eq!(big(7.0e33), "7.00Dc");
    }

    #[test]
    fn beyond_decillion_uses_alpha_suffix() {
        // 10^36 is the first step past Decillion.
        assert_eq!(big(1.0e36), "1.00aa");
        assert_eq!(big(3.5e36), "3.50aa");
        assert_eq!(big(1.0e39), "1.00ab");
        assert_eq!(big(1.0e42), "1.00ac");
    }

    #[test]
    fn alpha_suffix_phase_transitions() {
        // First lowercase pair (n=0) is "aa".
        assert_eq!(alpha_suffix(0), "aa");
        // Last lowercase pair (n=675) is "zz".
        assert_eq!(alpha_suffix(675), "zz");
        // Then uppercase-first pairs start at n=676 with "Aa".
        assert_eq!(alpha_suffix(676), "Aa");
        // Last uppercase-first pair (n=1351) is "Zz".
        assert_eq!(alpha_suffix(1351), "Zz");
        // Then lowercase triples begin at n=1352 with "aaa".
        assert_eq!(alpha_suffix(1352), "aaa");
    }

    #[test]
    fn alpha_suffix_within_phase_is_base26() {
        // n=1 → "ab", n=25 → "az", n=26 → "ba".
        assert_eq!(alpha_suffix(1), "ab");
        assert_eq!(alpha_suffix(25), "az");
        assert_eq!(alpha_suffix(26), "ba");
    }

    #[test]
    fn infinity_and_nan_render_as_question_mark() {
        assert_eq!(big(f64::INFINITY), "?");
        assert_eq!(big(f64::NEG_INFINITY), "?");
        assert_eq!(big(f64::NAN), "?");
    }

    #[test]
    fn big_mag_handles_unbounded_values() {
        use crate::bignum::Mag;
        // 10^36 → first alpha-suffix entry.
        assert_eq!(big_mag(Mag::from_f64(1.0e36)), "1.00aa");
        // 10^999 — past f64::MAX, but Mag handles it cleanly. Pick
        // a power that's an exact multiple of 3 so the mantissa is "1".
        let huge = Mag { log10: 999.0 };
        let s = big_mag(huge);
        assert!(!s.contains('?'), "got {s}");
        assert!(s.starts_with("1.00"), "got {s}");
    }

    #[test]
    fn big_mag_handles_zero_and_small() {
        use crate::bignum::Mag;
        assert_eq!(big_mag(Mag::ZERO), "0");
        assert_eq!(big_mag(Mag::from_f64(42.0)), "42");
        assert_eq!(big_mag(Mag::from_f64(1500.0)), "1.50k");
    }

    #[test]
    fn huge_but_finite_number_does_not_print_question_mark() {
        // 1e200 would have overflowed the old fixed-suffix table and
        // produced a stretch of trailing zeros in front of "Dc". With
        // the alpha tail it formats compactly with a real suffix.
        let s = big(1.0e200);
        assert!(!s.contains('?'), "got {s}");
        assert!(s.ends_with(char::is_alphabetic), "got {s}");
    }
}
