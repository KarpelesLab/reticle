//! `$random` and the `$dist_*` functions, as IEEE 1364-2005 §17.9 gives
//! them.
//!
//! The standard does not leave the generator to the implementation: it
//! prints C source for it (`rtl_dist_uniform` and the six other
//! `rtl_dist_*`, which `$random` is a case of), so that a seed variable
//! takes the same values in every simulator. This is that code, with C's
//! 32-bit `long` as `i32` and its casts spelled out. That it is the code
//! xsim 2026.1 runs was measured rather than read: every value in the tests
//! below is what xsim printed for the same calls, `$random(seed)` from seed
//! 1 giving -2147414528 and leaving the seed 69070, and `$dist_normal`,
//! `$dist_exponential`, `$dist_poisson`, `$dist_chi_square`, `$dist_t` and
//! `$dist_erlang` from seed 11 giving xsim's first three values and seeds.
//!
//! A diagnostic the standard's code prints (a mean that is not positive,
//! say) is returned as text for the caller to report; the result is then
//! the standard's 0 and the seed is left alone.

/// The uniform deviate between `start` and `end` all the others draw on;
/// it advances `seed` once.
fn uniform(seed: &mut i32, start: i32, end: i32) -> f64 {
    let d = 0.000_000_119_209_289_550_781_25;
    if *seed == 0 {
        *seed = 259_341_593;
    }
    let (a, b) = if start >= end {
        (0.0, 2_147_483_647.0)
    } else {
        (f64::from(start), f64::from(end))
    };
    *seed = seed.wrapping_mul(69_069).wrapping_add(1);
    // The new seed's top 23 bits become the mantissa of a float in [1, 2).
    let temp = (seed.cast_unsigned() >> 9) | 0x3f80_0000;
    let c = f64::from(f32::from_bits(temp));
    let c = c + c * d;
    (b - a) * (c - 1.0) + a
}

/// C's `(long) r` of a real the standard has already kept in range.
fn to_long(r: f64) -> i64 {
    // The cast truncates towards zero, as C's does, and saturates
    // where C's would be undefined.
    #[allow(clippy::cast_possible_truncation)]
    let v = r as i64;
    v
}

/// The low 32 bits, C's conversion of a wider integer to `long`.
fn wrap(v: i64) -> i32 {
    #[allow(clippy::cast_possible_truncation)]
    let w = v as i32;
    w
}

/// The `floor`-like cast the uniform functions use: down for negatives.
fn floor_long(r: f64) -> i64 {
    if r >= 0.0 {
        to_long(r)
    } else {
        to_long(r - 1.0)
    }
}

/// Rounds half away from zero, as the `rtl_dist_*` functions do.
fn round_long(r: f64) -> i32 {
    if r >= 0.0 {
        wrap(to_long(r + 0.5))
    } else {
        wrap(-to_long(-r + 0.5))
    }
}

/// `$dist_uniform(seed, start, end)`: an integer in `start..=end`.
pub(crate) fn dist_uniform(seed: &mut i32, start: i32, end: i32) -> i32 {
    if start >= end {
        return start;
    }
    let (start, end) = (i64::from(start), i64::from(end));
    let i = if end != i64::from(i32::MAX) {
        let end = end + 1;
        let r = uniform(seed, wrap(start), wrap(end));
        floor_long(r).clamp(start, end - 1)
    } else if start != i64::from(i32::MIN) {
        let start = start - 1;
        let r = uniform(seed, wrap(start), wrap(end)) + 1.0;
        floor_long(r).clamp(start + 1, end)
    } else {
        let r = (uniform(seed, wrap(start), wrap(end)) + 2_147_483_648.0) / 4_294_967_295.0;
        floor_long(r * 4_294_967_296.0 - 2_147_483_648.0)
    };
    wrap(i)
}

/// `$random(seed)`: a uniform 32-bit signed value.
pub(crate) fn random(seed: &mut i32) -> i32 {
    dist_uniform(seed, i32::MIN, i32::MAX)
}

/// The standard's polar normal deviate.
fn normal(seed: &mut i32, mean: i32, deviation: i32) -> f64 {
    let (mut v1, mut s) = (0.0, 1.0);
    while s >= 1.0 || s == 0.0 {
        v1 = uniform(seed, -1, 1);
        let v2 = uniform(seed, -1, 1);
        s = v1 * v1 + v2 * v2;
    }
    let s = v1 * (-2.0 * s.ln() / s).sqrt();
    s * f64::from(deviation) + f64::from(mean)
}

/// The standard's exponential deviate.
fn exponential(seed: &mut i32, mean: i32) -> f64 {
    let n = uniform(seed, 0, 1);
    if n == 0.0 {
        n
    } else {
        -n.ln() * f64::from(mean)
    }
}

/// The standard's chi-square deviate.
fn chi_square(seed: &mut i32, deg_of_free: i32) -> f64 {
    let mut x = if deg_of_free % 2 != 0 {
        let n = normal(seed, 0, 1);
        n * n
    } else {
        0.0
    };
    let mut k = 2;
    while k <= deg_of_free {
        x += 2.0 * exponential(seed, 1);
        k += 2;
    }
    x
}

/// One of the `$dist_*` functions by name, on `seed` and the remaining
/// arguments; `None` for a name that is not one. The result pairs the
/// value with the standard's complaint, if it made one.
pub(crate) fn dist(
    name: &str,
    seed: &mut i32,
    args: &[i32],
) -> Option<(i32, Option<&'static str>)> {
    let arg = |i: usize| args.get(i).copied().unwrap_or(0);
    let checked = |ok: bool, msg: &'static str, f: &mut dyn FnMut() -> i32| {
        if ok { (f(), None) } else { (0, Some(msg)) }
    };
    Some(match name {
        "$random" => (random(seed), None),
        "$dist_uniform" => {
            let (start, end) = (arg(0), arg(1));
            let warn = (start >= end).then_some("$dist_uniform: start is not less than end");
            (dist_uniform(seed, start, end), warn)
        }
        "$dist_normal" => (round_long(normal(seed, arg(0), arg(1))), None),
        "$dist_exponential" => checked(
            arg(0) > 0,
            "exponential distribution must have a positive mean",
            &mut || round_long(exponential(seed, arg(0))),
        ),
        "$dist_poisson" => checked(
            arg(0) > 0,
            "Poisson distribution must have a positive mean",
            &mut || {
                let mut n = 0;
                let p = (-f64::from(arg(0))).exp();
                let mut q = uniform(seed, 0, 1);
                while p < q {
                    n += 1;
                    q *= uniform(seed, 0, 1);
                }
                n
            },
        ),
        "$dist_chi_square" => checked(
            arg(0) > 0,
            "chi-square distribution must have a positive degree of freedom",
            &mut || round_long(chi_square(seed, arg(0))),
        ),
        "$dist_t" => checked(
            arg(0) > 0,
            "t distribution must have a positive degree of freedom",
            &mut || {
                let chi2 = chi_square(seed, arg(0));
                let root = (chi2 / f64::from(arg(0))).sqrt();
                round_long(normal(seed, 0, 1) / root)
            },
        ),
        "$dist_erlang" => checked(
            arg(0) > 0,
            "k-stage Erlangian distribution must have a positive k",
            &mut || {
                let (k, mean) = (arg(0), arg(1));
                let mut x = 1.0;
                for _ in 0..k {
                    x *= uniform(seed, 0, 1);
                }
                round_long(-f64::from(mean) * x.ln() / f64::from(k))
            },
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `name` three times from seed 11 and returns each value with
    /// the seed it leaves.
    fn three(name: &str, args: &[i32]) -> Vec<(i32, i32)> {
        let mut seed = 11;
        (0..3)
            .map(|_| {
                let (v, _) = dist(name, &mut seed, args).unwrap();
                (v, seed)
            })
            .collect()
    }

    /// Every value here is what xsim 2026.1 printed for the same calls.
    /// Would catch: any departure from the standard's arithmetic — the
    /// float mantissa trick, the seed recurrence, the rounding, the order
    /// in which the composite distributions draw. Would not catch: a
    /// fault that only shows at seeds or arguments not listed.
    #[test]
    fn values_are_the_standards_and_xsims() {
        let mut seed = 1;
        assert_eq!((random(&mut seed), seed), (-2_147_414_528, 69070));
        assert_eq!((random(&mut seed), seed), (-1_671_855_048, 475_628_535));
        let mut seed = 0;
        assert_eq!((random(&mut seed), seed), (303_379_748, -1_844_104_698));
        let mut seed = -7;
        assert_eq!((random(&mut seed), seed), (2_147_000_831, -483_482));
        let mut seed = 5;
        assert_eq!((dist_uniform(&mut seed, 0, 99), seed), (0, 345_346));
        let mut seed = 3;
        assert_eq!((dist_uniform(&mut seed, -10, 10), seed), (-10, 207_208));
        assert_eq!(dist_uniform(&mut seed, 10, 5), 10);
        assert_eq!(seed, 207_208);
        assert_eq!(
            three("$dist_normal", &[100, 15]),
            [
                (80, 1_498_760_199),
                (92, -1_033_098_003),
                (79, 1_568_767_427)
            ]
        );
        assert_eq!(
            three("$dist_exponential", &[50]),
            [(432, 759_760), (76, 936_255_889), (63, 1_230_388_766)]
        );
        assert_eq!(
            three("$dist_poisson", &[7]),
            [(0, 759_760), (5, 1_540_686_538), (5, 91_847_284)]
        );
        assert_eq!(
            three("$dist_chi_square", &[4]),
            [(20, 936_255_889), (5, 1_498_760_199), (5, -1_033_098_003)]
        );
        assert_eq!(
            three("$dist_t", &[3]),
            [
                (1, 1_540_686_538),
                (-1, -1_832_490_689),
                (-1, 1_885_367_116)
            ]
        );
        assert_eq!(
            three("$dist_erlang", &[3, 20]),
            [
                (76, 1_230_388_766),
                (22, -1_033_098_003),
                (14, -237_528_024)
            ]
        );
        let mut seed = 99;
        assert_eq!(dist("$dist_poisson", &mut seed, &[30]), Some((17, None)));
        assert_eq!(seed, 1_926_252_953);
        // A mean that is not positive: 0, the seed untouched, a warning.
        let (v, warn) = dist("$dist_exponential", &mut seed, &[-1]).unwrap();
        assert_eq!((v, seed), (0, 1_926_252_953));
        assert!(warn.is_some());
        assert_eq!(dist("$dist_nothing", &mut seed, &[]), None);
    }
}
