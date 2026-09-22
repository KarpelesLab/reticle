//! Solving a PLL's dividers for a frequency.
//!
//! A design asks for a clock of so many MHz; the board gives it one of
//! some other frequency; a PLL sits between them, and what configures it
//! is three integers — a reference divider, a feedback divider and an
//! output divider — chosen from ranges the silicon allows, subject to the
//! phase detector and the VCO each staying inside their own frequency
//! windows. [`solve`] searches every legal combination the device
//! database describes and returns the one closest to the request, with
//! the frequency it really gives and the error.
//!
//! Nothing here knows a family. The ranges, the parameter names, how a
//! written value relates to the division it performs (iCE40 stores
//! `divisor - 1` in `DIVR` and `DIVF` and a power of two in `DIVQ`; ECP5
//! stores the divisor itself), where the feedback is taken from (the VCO
//! on iCE40 in `SIMPLE` mode, the output on ECP5 with `FEEDBK_PATH =
//! "CLKOP"`), the parameters that follow from the answer (`CLKOP_CPHASE`
//! tracking `CLKOP_DIV`, the loop filter band chosen by the phase
//! detector's frequency) — all of that is a [`PllShape`] read from the
//! `.dev` file. The formulas are therefore only the two a PLL has:
//!
//! ```text
//! feedback from the VCO:     pfd = in / R    vco = pfd * F    out = vco / O
//! feedback from the output:  pfd = in / R    out = pfd * F    vco = out * O
//! ```
//!
//! # Determinism and tie-breaking
//!
//! The search runs the reference divider, then the feedback divider,
//! then the output divider, each from its smallest value up, and only a
//! *strictly* smaller error replaces the best so far. Among equally good
//! settings the one with the smallest reference divider wins — the
//! highest phase-detector frequency, which is the lower-jitter choice —
//! and the result is the same on every platform, since it is plain IEEE
//! arithmetic on the same inputs in the same order.
//!
//! # Accuracy
//!
//! The answer is exact for the arithmetic the PLL itself performs: the
//! achieved frequency is `in * F / (R * O)` (or `in * F / R`), computed
//! in `f64`, and the search is exhaustive, so no better setting exists
//! within the ranges the database states. How close that is to the
//! request depends only on how rich those ranges are.
//! `measure_the_error_over_each_family_s_range` (an ignored test) asks
//! for every frequency in 0.1 MHz steps and reports:
//!
//! | PLL | reference | range asked | median | 90th percentile | worst |
//! |-----|-----------|-------------|--------|-----------------|-------|
//! | `SB_PLL40_CORE` | 12 MHz | 16–275 MHz | 0.39 % | 0.75 % | 1.1 % |
//! | `EHXPLLL` | 25 MHz | 10–400 MHz | 0.30 % | 1.4 % | 10.7 % |
//!
//! The ECP5's worst case is at the bottom of its range: the phase
//! detector must stay above 4 MHz (the database rounds the real 3.125 MHz
//! up, to stay safe with whole numbers), so from 25 MHz the reference
//! divider is at most 6 and a low output has few ratios to choose from.
//! Exact ratios — 48 MHz from 12, 125 MHz from 25 — come out exact. The
//! solution always states its error; the caller decides whether it is
//! enough.

use super::device::{PllDividerRole, PllFeedback, PllShape};

/// How many divider combinations [`solve`] will try before declaring a
/// `.dev` description too loose to search. The largest real one, the
/// ECP5's, is 128 × 80 × 128 ≈ 1.3 million.
const MAX_COMBINATIONS: u64 = 20_000_000;

/// One solved PLL configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct PllSolution {
    /// The primitive it configures.
    pub primitive: String,
    /// The reference frequency, in MHz.
    pub input_mhz: f64,
    /// The frequency asked for, in MHz.
    pub requested_mhz: f64,
    /// The frequency this setting produces, in MHz.
    pub achieved_mhz: f64,
    /// The phase detector's frequency, in MHz.
    pub pfd_mhz: f64,
    /// The VCO's frequency, in MHz.
    pub vco_mhz: f64,
    /// The division each divider performs, in role order: reference,
    /// feedback, output.
    pub divisions: [u64; 3],
    /// Every parameter the solution sets, with its value: the three
    /// dividers as written, then the derived parameters, then the banded
    /// ones, each in file order.
    pub params: Vec<(String, i64)>,
}

impl PllSolution {
    /// How far the achieved frequency is from the request, in MHz;
    /// positive when it is above.
    pub fn error_mhz(&self) -> f64 {
        self.achieved_mhz - self.requested_mhz
    }

    /// The same error in parts per million of the request.
    pub fn error_ppm(&self) -> f64 {
        if self.requested_mhz == 0.0 {
            return 0.0;
        }
        self.error_mhz() / self.requested_mhz * 1e6
    }

    /// The value of one parameter the solution sets.
    pub fn param(&self, name: &str) -> Option<i64> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| *value)
    }

    /// One line saying what was asked, what was reached and how.
    ///
    /// ```
    /// # use reticle::fpga::{pll, target};
    /// let device = target("ice40-hx1k-tq144").unwrap();
    /// let shape = &device.clock_resources.plls[0];
    /// let solution = pll::solve(shape, 12.0, 48.0).unwrap();
    /// assert_eq!(
    ///     solution.describe(),
    ///     "SB_PLL40_CORE: 12.000 MHz -> 48.000 MHz (asked 48.000 MHz, error +0.0 ppm; \
    ///      pfd 12.000 MHz, vco 768.000 MHz; DIVR=0 DIVF=63 DIVQ=4 FILTER_RANGE=1)"
    /// );
    /// ```
    pub fn describe(&self) -> String {
        let params: Vec<String> = self
            .params
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        format!(
            "{}: {:.3} MHz -> {:.3} MHz (asked {:.3} MHz, error {:+.1} ppm; pfd {:.3} MHz, \
             vco {:.3} MHz; {})",
            self.primitive,
            self.input_mhz,
            self.achieved_mhz,
            self.requested_mhz,
            self.error_ppm(),
            self.pfd_mhz,
            self.vco_mhz,
            params.join(" ")
        )
    }
}

/// Finds the divider setting of `shape` whose output is closest to
/// `target_mhz` from a reference of `input_mhz`.
///
/// The error is a sentence for a diagnostic: the shape cannot be
/// configured at all, the reference is outside its input range, or no
/// combination keeps both the phase detector and the VCO in range.
///
/// ```
/// # use reticle::fpga::{pll, target};
/// // The ULX3S's 25 MHz oscillator to the 125 MHz of a DVI pixel clock
/// // times five: exact on the ECP5.
/// let device = target("ecp5-45f-CABGA381").unwrap();
/// let shape = &device.clock_resources.plls[0];
/// let solution = pll::solve(shape, 25.0, 125.0).unwrap();
/// assert_eq!(solution.achieved_mhz, 125.0);
/// assert_eq!(solution.param("CLKFB_DIV"), Some(5));
/// ```
pub fn solve(shape: &PllShape, input_mhz: f64, target_mhz: f64) -> Result<PllSolution, String> {
    if !shape.is_configurable() {
        return Err(format!(
            "the device database describes `{}` without its dividers, VCO range and \
             `ref` / `out` ports, so it cannot be configured",
            shape.name
        ));
    }
    if !(input_mhz.is_finite() && input_mhz > 0.0 && target_mhz.is_finite() && target_mhz > 0.0) {
        return Err(format!(
            "{input_mhz} MHz to {target_mhz} MHz is not a frequency pair a PLL can be asked for"
        ));
    }
    if let Some((lo, hi)) = shape.input_mhz
        && !within(input_mhz, lo, hi)
    {
        return Err(format!(
            "`{}` takes a reference of {lo} to {hi} MHz, and the input clock is {input_mhz:.3} MHz",
            shape.name
        ));
    }
    let (Some(r), Some(f), Some(o), Some((vco_lo, vco_hi))) = (
        shape.divider(PllDividerRole::Reference),
        shape.divider(PllDividerRole::Feedback),
        shape.divider(PllDividerRole::Output),
        shape.vco_mhz,
    ) else {
        unreachable!("is_configurable checked all four");
    };
    let span = |min: u32, max: u32| u64::from(max.saturating_sub(min)) + 1;
    let combinations = span(r.min, r.max)
        .saturating_mul(span(f.min, f.max))
        .saturating_mul(span(o.min, o.max));
    if combinations > MAX_COMBINATIONS {
        return Err(format!(
            "`{}` describes {combinations} divider settings, more than the {MAX_COMBINATIONS} \
             Reticle will search",
            shape.name
        ));
    }

    let mut best: Option<Candidate> = None;
    for rv in r.min..=r.max {
        let rd = r.divisor(rv);
        if rd == 0 {
            continue;
        }
        let pfd = input_mhz / to_f64(rd);
        if let Some((lo, hi)) = shape.pfd_mhz
            && !within(pfd, lo, hi)
        {
            continue;
        }
        if !shape
            .bands
            .iter()
            .all(|(_, bands)| band_of(bands, pfd).is_some())
        {
            continue;
        }
        for fv in f.min..=f.max {
            let fd = f.divisor(fv);
            if fd == 0 {
                continue;
            }
            for ov in o.min..=o.max {
                let od = o.divisor(ov);
                if od == 0 {
                    continue;
                }
                let (vco, out) = match shape.feedback {
                    PllFeedback::Vco => {
                        let vco = pfd * to_f64(fd);
                        (vco, vco / to_f64(od))
                    }
                    PllFeedback::Output => {
                        let out = pfd * to_f64(fd);
                        (out * to_f64(od), out)
                    }
                };
                if !within(vco, vco_lo, vco_hi) {
                    continue;
                }
                let error = (out - target_mhz).abs();
                if best.as_ref().is_none_or(|b| error < b.error) {
                    best = Some(Candidate {
                        values: [rv, fv, ov],
                        divisions: [rd, fd, od],
                        pfd,
                        vco,
                        out,
                        error,
                    });
                }
                // With the feedback taken from the output, the output
                // divider only moves the VCO: the first that puts it in
                // range is as good as any.
                if shape.feedback == PllFeedback::Output {
                    break;
                }
            }
        }
    }
    let Some(best) = best else {
        return Err(format!(
            "no setting of `{}` keeps its phase detector and VCO in range for {input_mhz:.3} MHz \
             in",
            shape.name
        ));
    };

    let mut params = Vec::new();
    for (divider, value) in [r, f, o].into_iter().zip(best.values) {
        params.push((divider.param.clone(), i64::from(value)));
    }
    for (param, role, offset) in &shape.derived {
        let index = match role {
            PllDividerRole::Reference => 0,
            PllDividerRole::Feedback => 1,
            PllDividerRole::Output => 2,
        };
        params.push((param.clone(), i64::from(best.values[index]) + offset));
    }
    for (param, bands) in &shape.bands {
        if let Some(value) = band_of(bands, best.pfd) {
            params.push((param.clone(), value));
        }
    }
    Ok(PllSolution {
        primitive: shape.name.clone(),
        input_mhz,
        requested_mhz: target_mhz,
        achieved_mhz: best.out,
        pfd_mhz: best.pfd,
        vco_mhz: best.vco,
        divisions: best.divisions,
        params,
    })
}

/// The best setting found so far.
struct Candidate {
    /// The values written to the reference, feedback and output fields.
    values: [u32; 3],
    /// The divisions those values perform.
    divisions: [u64; 3],
    pfd: f64,
    vco: f64,
    out: f64,
    error: f64,
}

/// True when `value` lies in `[lo, hi]` MHz.
fn within(value: f64, lo: u32, hi: u32) -> bool {
    value >= f64::from(lo) && value <= f64::from(hi)
}

/// The value of the band `pfd` falls in: the first whose exclusive upper
/// bound it is below.
fn band_of(bands: &[(u32, i64)], pfd: f64) -> Option<i64> {
    bands
        .iter()
        .find(|(upper, _)| pfd < f64::from(*upper))
        .map(|(_, value)| *value)
}

/// A divisor as a float. Every divisor a real PLL has is far below 2^32,
/// where the conversion is exact.
fn to_f64(value: u64) -> f64 {
    let value = u32::try_from(value).unwrap_or(u32::MAX);
    f64::from(value)
}

/// Megahertz to whole hertz, rounded to the nearest.
///
/// This is the one float-to-integer conversion the clock code makes, so
/// the cast is here with its argument: the value is checked finite and
/// positive, and clamped far below 2^64, before it is truncated.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn to_hz(mhz: f64) -> u64 {
    let hz = (mhz * 1e6).round();
    if hz.is_finite() && hz > 0.0 {
        hz.min(1e18) as u64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::target;

    fn shape(device: &str, name: &str) -> PllShape {
        target(device)
            .unwrap()
            .clock_resources
            .plls
            .iter()
            .find(|p| p.name == name)
            .unwrap()
            .clone()
    }

    #[test]
    fn ice40_reaches_the_icepll_answers() {
        // The settings `icepll -i 12 -o <f>` prints for the iCEstick's
        // oscillator, which is the reference every published iCE40
        // design is checked against.
        let pll = shape("ice40-hx1k-tq144", "SB_PLL40_CORE");
        let cases: [(f64, i64, i64, i64, i64, f64); 3] = [
            // (asked, DIVR, DIVF, DIVQ, FILTER_RANGE, achieved)
            (48.0, 0, 63, 4, 1, 48.0),
            (100.0, 0, 66, 3, 1, 100.5),
            (36.0, 0, 47, 4, 1, 36.0),
        ];
        for (asked, divr, divf, divq, filter, achieved) in cases {
            let s = solve(&pll, 12.0, asked).unwrap();
            assert_eq!(s.param("DIVR"), Some(divr), "{}", s.describe());
            assert_eq!(s.param("DIVF"), Some(divf), "{}", s.describe());
            assert_eq!(s.param("DIVQ"), Some(divq), "{}", s.describe());
            assert_eq!(s.param("FILTER_RANGE"), Some(filter), "{}", s.describe());
            assert!((s.achieved_mhz - achieved).abs() < 1e-9, "{}", s.describe());
            // The windows the part imposes.
            assert!((533.0..=1066.0).contains(&s.vco_mhz));
            assert!((10.0..=133.0).contains(&s.pfd_mhz));
        }
    }

    #[test]
    fn ecp5_reaches_exact_multiples_and_reports_its_error() {
        let pll = shape("ecp5-45f-CABGA381", "EHXPLLL");
        // 25 MHz to the 125 MHz a DVI serialiser wants: exact.
        let s = solve(&pll, 25.0, 125.0).unwrap();
        assert_eq!(s.achieved_mhz, 125.0);
        assert_eq!(s.error_ppm(), 0.0);
        // CLKOP_CPHASE follows CLKOP_DIV, one less, for no phase shift.
        assert_eq!(s.param("CLKOP_CPHASE"), s.param("CLKOP_DIV").map(|d| d - 1));
        assert!((400.0..=800.0).contains(&s.vco_mhz));

        // 25 MHz to 74.25 MHz, the 720p pixel clock. 74.25 / 25 is
        // 297 / 100, and the phase detector must stay above 4 MHz, so
        // the reference divider is at most 6 and every ratio it allows
        // rounds to 3: the answer is 75 MHz, a 1 % error, and it is
        // stated rather than hidden. (That is why ULX3S DVI designs run
        // 720p at 75 MHz.)
        let s = solve(&pll, 25.0, 74.25).unwrap();
        assert_eq!(s.achieved_mhz, 75.0, "{}", s.describe());
        assert!((s.error_ppm() - 10_101.0).abs() < 1.0, "{}", s.describe());
        let expected = 25.0 * (s.divisions[1] as f64) / (s.divisions[0] as f64);
        assert!((s.achieved_mhz - expected).abs() < 1e-9);
    }

    #[test]
    #[ignore = "a measurement, not a check: run with --ignored --nocapture"]
    fn measure_the_error_over_each_family_s_range() {
        for (device, name, input, lo, hi) in [
            ("ice40-hx1k-tq144", "SB_PLL40_CORE", 12.0, 16.0, 275.0),
            ("ecp5-45f-CABGA381", "EHXPLLL", 25.0, 10.0, 400.0),
        ] {
            let pll = shape(device, name);
            let mut errors: Vec<f64> = Vec::new();
            let mut target: f64 = lo;
            while target <= hi {
                if let Ok(s) = solve(&pll, input, target) {
                    errors.push(s.error_ppm().abs());
                }
                target += 0.1;
            }
            errors.sort_by(f64::total_cmp);
            let pick = |percent: usize| errors[(errors.len() - 1) * percent / 100];
            println!(
                "{name} from {input} MHz, {lo}..{hi} MHz in 0.1 MHz steps ({} requests): \
                 median {:.0} ppm, 90th percentile {:.0} ppm, worst {:.0} ppm",
                errors.len(),
                pick(50),
                pick(90),
                pick(100)
            );
        }
    }

    #[test]
    fn the_search_is_deterministic_and_prefers_a_small_reference_divider() {
        let pll = shape("ecp5-45f-CABGA381", "EHXPLLL");
        let a = solve(&pll, 25.0, 50.0).unwrap();
        let b = solve(&pll, 25.0, 50.0).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.param("CLKI_DIV"), Some(1));
        assert_eq!(a.param("CLKFB_DIV"), Some(2));
    }

    #[test]
    fn what_cannot_be_reached_is_said() {
        let pll = shape("ice40-hx1k-tq144", "SB_PLL40_CORE");
        // Below the reference range.
        let err = solve(&pll, 5.0, 48.0).unwrap_err();
        assert!(err.contains("takes a reference of 10 to 133 MHz"), "{err}");
        // A shape described without its dividers.
        let bare = PllShape::new("BARE");
        let err = solve(&bare, 12.0, 48.0).unwrap_err();
        assert!(err.contains("cannot be configured"), "{err}");
        // Nonsense.
        assert!(solve(&pll, 12.0, 0.0).is_err());
    }

    #[test]
    fn a_far_request_gets_the_nearest_legal_setting() {
        // The iCE40 cannot go below 533 / 64 MHz; asking for 1 MHz gives
        // the closest it has and says how far off it is.
        let pll = shape("ice40-hx1k-tq144", "SB_PLL40_CORE");
        let s = solve(&pll, 12.0, 1.0).unwrap();
        assert!(s.achieved_mhz > 8.0, "{}", s.describe());
        assert!(s.error_ppm() > 1e6);
    }
}
