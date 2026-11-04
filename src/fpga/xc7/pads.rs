//! The IO standards of a 7-series high-range bank, and the rules Vivado's
//! design rule checks make of a bank's pads.
//!
//! This is only what the part and the tool say about a pad, with no database
//! and no design in it: `fpga::xray::io` turns a [`Standard`] into
//! bits, and [`check_pads`] says which set of pads may not share a bank.
//!
//! **Measured, all of it**, on the XC7A100T in FTG256, whose banks are all
//! high-range, by `tools/vivado-parity/gen_io.py`: every standard Vivado 2026.1
//! lists for a port (`list_property_value IOSTANDARD`), 37 of them, in every
//! direction. `docs/fpga-vivado-parity.md` has what was asked and what came
//! back; `testdata/fpga/vivado/io/` keeps the answers.
//!
//! The two supplies in [`Standard`] are a fit, and how good is the point:
//! `pairs.tcl` asked Vivado's DRC about 11,236 ordered pairs of (standard,
//! direction) in one bank, and [`Standard::vcco_of`] and [`Standard::vref_of`]
//! say what it said for each of them, `BIVC-1` and `BIVR-1` alike.

use std::collections::BTreeMap;

use crate::source::Span;

/// Which way a pad is used. An `IOBUF` is both, and the rules read that as an
/// output that also receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Way {
    /// The pad drives the fabric.
    In,
    /// The fabric drives the pad.
    Out,
    /// Both.
    InOut,
}

/// An IO standard of a high-range bank. Millivolts throughout, since `1.35` is
/// one of the values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Standard {
    /// As constraints spell it.
    pub name: &'static str,
    /// Uses a pair of balls: `IOSTDTYPE-1` refuses it on one, and refuses
    /// anything else on a pair.
    pub differential: bool,
    /// May be bidirectional: `Place 30-49` refuses an `IOBUF` otherwise.
    pub bidirectional: bool,
    /// The bank supply it needs when it needs one.
    pub vcco: u32,
    /// Needs [`Standard::vcco`] even as an input: a reference-voltage or
    /// differential receiver does not, in `BIVC-1`'s view.
    pub input_needs_vcco: bool,
    /// The reference voltage its single-ended receiver compares with.
    pub vref: Option<u32>,
    /// Takes `IN_TERM`: `InOutTerm-9` refuses it otherwise.
    pub in_term: bool,
    /// `DIFF_TERM` sets bits. On any other standard Vivado says nothing of it
    /// and writes the same bits.
    pub diff_term: bool,
    /// The drives it has, in mA. Vivado refuses any other (`Common 17-69`); on
    /// an empty list it warns of any (`Vivado 12-4708`) and ignores it.
    pub drives: &'static [u32],
    /// `SLEW` means something. Otherwise Vivado warns (`Vivado 12-4709`) and
    /// ignores it.
    pub slew: bool,
}

const fn single(name: &'static str, vcco: u32, drives: &'static [u32]) -> Standard {
    Standard {
        name,
        differential: false,
        bidirectional: true,
        vcco,
        input_needs_vcco: true,
        vref: None,
        in_term: false,
        diff_term: false,
        drives,
        slew: true,
    }
}

const fn referenced(name: &'static str, vcco: u32, vref: u32, bidirectional: bool) -> Standard {
    Standard {
        name,
        differential: false,
        bidirectional,
        vcco,
        input_needs_vcco: false,
        vref: Some(vref),
        in_term: true,
        diff_term: false,
        drives: &[],
        slew: true,
    }
}

const fn pseudo(name: &'static str, vcco: u32, bidirectional: bool) -> Standard {
    Standard {
        name,
        differential: true,
        bidirectional,
        vcco,
        input_needs_vcco: false,
        vref: None,
        in_term: true,
        diff_term: false,
        drives: &[],
        slew: true,
    }
}

const fn true_differential(
    name: &'static str,
    vcco: u32,
    bidirectional: bool,
    diff_term: bool,
) -> Standard {
    Standard {
        name,
        differential: true,
        bidirectional,
        vcco,
        input_needs_vcco: false,
        vref: None,
        in_term: false,
        diff_term,
        drives: &[],
        slew: false,
    }
}

/// `HSUL_12` and its pair take no `IN_TERM`, unlike the others of their kind.
const fn no_term(mut standard: Standard) -> Standard {
    standard.in_term = false;
    standard
}

/// A pair's `MOBILE_DDR`, which needs its supply as an input, as `MOBILE_DDR`
/// does.
const fn needs_vcco(mut standard: Standard) -> Standard {
    standard.input_needs_vcco = true;
    standard
}

/// `PCI33_3`: no slew.
const fn no_slew(mut standard: Standard) -> Standard {
    standard.slew = false;
    standard
}

/// Every standard `list_property_value IOSTANDARD` gives for a port of this
/// part, and no other: the `_DCI` and `T_DCI` forms are the high-performance
/// banks', and an XC7A100T has none.
pub const STANDARDS: [Standard; 37] = [
    single("LVTTL", 3300, &[4, 8, 12, 16, 24]),
    single("LVCMOS33", 3300, &[4, 8, 12, 16]),
    single("LVCMOS25", 2500, &[4, 8, 12, 16]),
    single("LVCMOS18", 1800, &[4, 8, 12, 16, 24]),
    single("LVCMOS15", 1500, &[4, 8, 12, 16]),
    single("LVCMOS12", 1200, &[4, 8, 12]),
    no_term(referenced("HSUL_12", 1200, 600, true)),
    referenced("HSTL_I", 1500, 750, false),
    referenced("HSTL_II", 1500, 750, true),
    referenced("HSTL_I_18", 1800, 900, false),
    referenced("HSTL_II_18", 1800, 900, true),
    referenced("SSTL18_I", 1800, 900, false),
    referenced("SSTL18_II", 1800, 900, true),
    referenced("SSTL15", 1500, 750, true),
    referenced("SSTL15_R", 1500, 750, true),
    referenced("SSTL135", 1350, 675, true),
    referenced("SSTL135_R", 1350, 675, true),
    pseudo("DIFF_HSTL_I", 1500, false),
    pseudo("DIFF_HSTL_II", 1500, true),
    pseudo("DIFF_HSTL_I_18", 1800, false),
    pseudo("DIFF_HSTL_II_18", 1800, true),
    pseudo("DIFF_SSTL18_I", 1800, false),
    pseudo("DIFF_SSTL18_II", 1800, true),
    pseudo("DIFF_SSTL15", 1500, true),
    pseudo("DIFF_SSTL15_R", 1500, true),
    pseudo("DIFF_SSTL135", 1350, true),
    pseudo("DIFF_SSTL135_R", 1350, true),
    no_term(pseudo("DIFF_HSUL_12", 1200, true)),
    no_slew(single("PCI33_3", 3300, &[])),
    single("MOBILE_DDR", 1800, &[]),
    no_term(needs_vcco(pseudo("DIFF_MOBILE_DDR", 1800, true))),
    true_differential("BLVDS_25", 2500, true, false),
    true_differential("LVDS_25", 2500, true, true),
    true_differential("RSDS_25", 2500, false, true),
    true_differential("TMDS_33", 3300, false, false),
    true_differential("MINI_LVDS_25", 2500, true, true),
    true_differential("PPDS_25", 2500, false, true),
];

/// The standard of that name.
pub fn standard(name: &str) -> Option<&'static Standard> {
    STANDARDS.iter().find(|s| s.name == name)
}

/// What Vivado reads as a true differential output standard, of which
/// `DIFFSTDLIMIT-1` allows a bank two. `BLVDS_25` is not one: `OUT_DIFF` makes
/// it from two single-ended drivers.
pub fn is_true_differential_output(name: &str) -> bool {
    matches!(
        name,
        "LVDS_25" | "RSDS_25" | "MINI_LVDS_25" | "PPDS_25" | "TMDS_33"
    )
}

impl Standard {
    /// The bank supply a pad of this standard asks for, if it asks.
    ///
    /// `in_term` and `diff_term` are whether a termination is stated: one
    /// terminates to the bank's supply. **`IN_TERM` was added to this after the
    /// fit, on `sstl15_in_term_lvcmos33_out`:** the pair matrix was asked
    /// without it, and the first reading had a referenced input needing no
    /// supply whatever it said.
    pub fn vcco_of(&self, way: Way, in_term: bool, diff_term: bool) -> Option<u32> {
        let terminated = in_term || (diff_term && self.diff_term);
        (way != Way::In || self.input_needs_vcco || terminated).then_some(self.vcco)
    }

    /// The reference voltage a pad of this standard asks for, if it asks. A
    /// differential receiver compares its two balls, so none.
    pub fn vref_of(&self, way: Way) -> Option<u32> {
        if way == Way::Out { None } else { self.vref }
    }
}

/// One pad, for [`check_pads`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pad {
    /// Its port, for a message.
    pub port: String,
    /// The ball, for a message.
    pub ball: String,
    /// The bank the ball is in.
    pub bank: String,
    /// The ball can be its bank's `VREF` pin.
    pub vref_ball: bool,
    /// The standard's name, which may not be one of [`STANDARDS`].
    pub standard: String,
    /// Which way.
    pub way: Way,
    /// A pair of balls, from a differential buffer.
    pub pair: bool,
    /// `IN_TERM`, if a value other than `NONE` is stated.
    pub in_term: Option<String>,
    /// `DIFF_TERM` is stated.
    pub diff_term: bool,
    /// An input-only buffer's `IBUFDISABLE` is a signal, and the rule that
    /// says so: `REQP-1964` of an `IBUF_IBUFDISABLE`, `REQP-1965` of an
    /// `IBUF_INTERMDISABLE`.
    pub ibufdisable: Option<&'static str>,
    /// Where it was asked for.
    pub span: Span,
}

/// Something Vivado refuses, under the name it refuses it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The rule: `BIVC-1`, `Place 30-49`.
    pub rule: &'static str,
    /// What is wrong.
    pub message: String,
    /// Where.
    pub span: Span,
}

/// The values `IVREF-2` allows, in millivolts: `0.5` and `1.2` were refused
/// with that name, and these four written. The value then **makes no
/// difference to the bits at all**: see `xray::io`.
pub const INTERNAL_VREF: [u32; 4] = [600, 675, 750, 900];

fn volts(millivolts: u32) -> String {
    format!("{:.3}", f64::from(millivolts) / 1000.0)
}

/// Everything Vivado refuses of these pads, in pad order and then bank order.
/// `internal_vref` is the banks a constraint gave `INTERNAL_VREF`, with its
/// value in millivolts and where that was said.
pub fn check_pads(pads: &[Pad], internal_vref: &[(String, u32, Span)]) -> Vec<Refusal> {
    let mut out = Vec::new();
    for pad in pads {
        let Some(std) = standard(&pad.standard) else {
            out.push(Refusal {
                rule: "Common 17-69",
                message: format!(
                    "`{}` is not an IO standard of this part's high-range banks: {} of them are \
                     known, and all were measured",
                    pad.standard,
                    STANDARDS.len()
                ),
                span: pad.span,
            });
            continue;
        };
        if std.differential != pad.pair {
            let (rule, needs, is) = if std.differential {
                ("IOSTDTYPE-1", "Differential", "Single-Ended")
            } else {
                ("Place 30-64", "Single-Ended", "Differential")
            };
            out.push(Refusal {
                rule,
                message: format!(
                    "port `{}` is {is} but has the IO standard `{}`, which can only be {needs}",
                    pad.port, pad.standard
                ),
                span: pad.span,
            });
        }
        // `REQP-1964`: measured on an `IBUF_IBUFDISABLE` and an
        // `IBUF_INTERMDISABLE`; an `IOBUF_INTERMDISABLE` is allowed, and so is
        // the pin tied either way.
        if let Some(rule) = pad.ibufdisable
            && matches!(
                std.name,
                "LVCMOS12"
                    | "LVCMOS15"
                    | "LVCMOS18"
                    | "LVCMOS25"
                    | "LVCMOS33"
                    | "MOBILE_DDR"
                    | "DIFF_MOBILE_DDR"
                    | "PCI33_3"
                    | "LVTTL"
            )
        {
            out.push(Refusal {
                rule,
                message: format!(
                    "`{}` uses the input pin `IBUFDISABLE`, which `{}` does not support",
                    pad.port, pad.standard
                ),
                span: pad.span,
            });
        }
        if pad.way == Way::InOut && !std.bidirectional {
            out.push(Refusal {
                rule: "Place 30-49",
                message: format!(
                    "port `{}` is bidirectional, and `{}` supports inputs or outputs only",
                    pad.port, pad.standard
                ),
                span: pad.span,
            });
        }
        if let Some(term) = &pad.in_term {
            if pad.way == Way::Out {
                out.push(Refusal {
                    rule: "InOutTerm-7",
                    message: format!(
                        "only inputs can use `IN_TERM`, but output port `{}` has it set to `{term}`",
                        pad.port
                    ),
                    span: pad.span,
                });
            } else if matches!(term.as_str(), "UNTUNED_SPLIT_25" | "UNTUNED_SPLIT_75") {
                out.push(Refusal {
                    rule: "InOutTerm-3",
                    message: format!(
                        "this part does not support an `IN_TERM` of `{term}`, but port `{}` uses it",
                        pad.port
                    ),
                    span: pad.span,
                });
            } else if !std.in_term {
                out.push(Refusal {
                    rule: "InOutTerm-9",
                    message: format!(
                        "`{}` does not support `IN_TERM`, but port `{}` has it set to `{term}`",
                        pad.standard, pad.port
                    ),
                    span: pad.span,
                });
            }
        }
    }
    for (bank, millivolts, span) in internal_vref {
        if !INTERNAL_VREF.contains(millivolts) {
            out.push(Refusal {
                rule: "IVREF-2",
                message: format!(
                    "bank {bank} has `INTERNAL_VREF` set to {} V, and this part has 0.600, \
                     0.675, 0.750 and 0.900",
                    volts(*millivolts)
                ),
                span: *span,
            });
        }
    }
    let mut banks: BTreeMap<&str, Vec<&Pad>> = BTreeMap::new();
    for pad in pads {
        banks.entry(pad.bank.as_str()).or_default().push(pad);
    }
    for (bank, here) in &banks {
        check_bank(bank, here, internal_vref, &mut out);
    }
    out
}

fn check_bank(
    bank: &str,
    pads: &[&Pad],
    internal_vref: &[(String, u32, Span)],
    out: &mut Vec<Refusal>,
) {
    let known: Vec<(&Pad, &Standard)> = pads
        .iter()
        .filter_map(|pad| standard(&pad.standard).map(|s| (*pad, s)))
        .collect();
    let supply =
        |(pad, std): &(&Pad, &Standard)| std.vcco_of(pad.way, pad.in_term.is_some(), pad.diff_term);
    if let Some(first) = known.iter().find(|p| supply(p).is_some())
        && let Some(other) = known
            .iter()
            .find(|p| supply(p).is_some_and(|v| Some(v) != supply(first)))
    {
        out.push(Refusal {
            rule: "BIVC-1",
            message: format!(
                "conflicting Vcc voltages in bank {bank}: `{}` ({}, requiring VCCO={}) and `{}` \
                 ({}, requiring VCCO={})",
                other.0.port,
                other.1.name,
                volts(supply(other).unwrap_or(0)),
                first.0.port,
                first.1.name,
                volts(supply(first).unwrap_or(0))
            ),
            span: other.0.span,
        });
    }
    let reference = |(pad, std): &(&Pad, &Standard)| std.vref_of(pad.way);
    let referenced: Vec<&(&Pad, &Standard)> =
        known.iter().filter(|p| reference(p).is_some()).collect();
    if let Some(first) = referenced.first() {
        if let Some(other) = referenced.iter().find(|p| reference(p) != reference(first)) {
            out.push(Refusal {
                rule: "BIVR-1",
                message: format!(
                    "conflicting Vref voltages in bank {bank}: `{}` ({}, Vref={} V) and `{}` ({}, \
                     Vref={} V)",
                    other.0.port,
                    other.1.name,
                    volts(reference(other).unwrap_or(0)),
                    first.0.port,
                    first.1.name,
                    volts(reference(first).unwrap_or(0))
                ),
                span: other.0.span,
            });
        }
        if !internal_vref.iter().any(|(b, _, _)| b == bank)
            && let Some(ball) = pads.iter().find(|p| p.vref_ball)
        {
            out.push(Refusal {
                rule: "BIVRU-1",
                message: format!(
                    "bank {bank} has ports using a reference voltage and no `INTERNAL_VREF`, so \
                     its VREF ball {} must supply it, and `{}` is on it",
                    ball.ball, ball.port
                ),
                span: ball.span,
            });
        }
    }
    let mut outputs: Vec<&str> = pads
        .iter()
        .filter(|p| p.way != Way::In && is_true_differential_output(&p.standard))
        .map(|p| p.standard.as_str())
        .collect();
    outputs.sort_unstable();
    outputs.dedup();
    if outputs.len() > 2 {
        out.push(Refusal {
            rule: "DIFFSTDLIMIT-1",
            message: format!(
                "output ports in bank {bank} can use at most 2 distinct true differential \
                 standards, and this bank has {}: {}",
                outputs.len(),
                outputs.join(", ")
            ),
            span: pads[0].span,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let file = map.add("x.xdc", "").unwrap();
        Span::new(file, 0, 0)
    }

    fn pad(port: &str, bank: &str, standard: &str, way: Way) -> Pad {
        Pad {
            port: port.to_owned(),
            ball: format!("{port}0"),
            bank: bank.to_owned(),
            vref_ball: false,
            standard: standard.to_owned(),
            way,
            pair: self::standard(standard).is_some_and(|s| s.differential),
            in_term: None,
            diff_term: false,
            ibufdisable: None,
            span: span(),
        }
    }

    fn rules(pads: &[Pad]) -> Vec<&'static str> {
        check_pads(pads, &[]).iter().map(|r| r.rule).collect()
    }

    /// One line of each rule against Vivado's own refusal of the same thing,
    /// which is why the odd ones are here: an `SSTL15` input beside an
    /// `LVCMOS33` output is allowed and a termination on it is not. Would catch
    /// a reference-voltage input read as needing the bank's supply, or the two
    /// voltages compared as one rule. The whole matrix is in
    /// `tests/fpga_xray_pads.rs`, against `pairs.txt`.
    #[test]
    fn the_rules_are_vivados() {
        use Way::{In, InOut, Out};
        assert_eq!(
            rules(&[
                pad("a", "35", "LVCMOS33", In),
                pad("b", "35", "LVCMOS25", In)
            ]),
            ["BIVC-1"]
        );
        assert!(
            rules(&[
                pad("a", "35", "LVDS_25", In),
                pad("b", "35", "LVCMOS33", Out)
            ])
            .is_empty()
        );
        assert_eq!(
            rules(&[
                pad("a", "35", "LVDS_25", Out),
                pad("b", "35", "LVCMOS33", In)
            ]),
            ["BIVC-1"]
        );
        let mut termed = pad("a", "35", "LVDS_25", In);
        termed.diff_term = true;
        assert_eq!(
            rules(&[termed, pad("b", "35", "LVCMOS33", Out)]),
            ["BIVC-1"]
        );
        assert_eq!(
            rules(&[pad("a", "35", "SSTL15", In), pad("b", "35", "SSTL135", In)]),
            ["BIVR-1"]
        );
        assert!(
            rules(&[pad("a", "35", "SSTL15", In), pad("b", "35", "HSTL_I", In)]).is_empty(),
            "the same reference from two standards"
        );
        assert!(
            rules(&[
                pad("a", "35", "SSTL15", In),
                pad("b", "35", "LVCMOS33", Out)
            ])
            .is_empty()
        );
        assert!(
            rules(&[
                pad("a", "35", "LVCMOS33", In),
                pad("b", "34", "LVCMOS25", In)
            ])
            .is_empty(),
            "two banks"
        );
        assert_eq!(rules(&[pad("a", "35", "HSTL_I", InOut)]), ["Place 30-49"]);
        let mut se_on_pair = pad("a", "35", "SSTL15", In);
        se_on_pair.pair = true;
        assert_eq!(rules(&[se_on_pair]), ["Place 30-64"]);
        let mut pair_on_one = pad("a", "35", "LVDS_25", In);
        pair_on_one.pair = false;
        assert_eq!(rules(&[pair_on_one]), ["IOSTDTYPE-1"]);
        let mut term = pad("a", "35", "LVCMOS33", In);
        term.in_term = Some("UNTUNED_SPLIT_50".to_owned());
        assert_eq!(rules(&[term]), ["InOutTerm-9"]);
        let mut term = pad("a", "35", "SSTL15", In);
        term.in_term = Some("UNTUNED_SPLIT_75".to_owned());
        assert_eq!(rules(&[term]), ["InOutTerm-3"]);
        assert_eq!(
            rules(&[
                pad("a", "35", "LVDS_25", Out),
                pad("b", "35", "RSDS_25", Out),
                pad("c", "35", "PPDS_25", Out)
            ]),
            ["DIFFSTDLIMIT-1"]
        );
        assert!(
            rules(&[
                pad("a", "35", "LVDS_25", Out),
                pad("b", "35", "RSDS_25", Out),
                pad("c", "35", "BLVDS_25", Out)
            ])
            .is_empty(),
            "`BLVDS_25` is not counted"
        );
    }

    /// The `VREF` ball: refused while a bank needs it, and only then.
    #[test]
    fn a_vref_ball_is_free_only_with_an_internal_reference() {
        use Way::{In, Out};
        let mut ball = pad("v", "35", "LVCMOS15", In);
        ball.vref_ball = true;
        let sstl = pad("a", "35", "SSTL15", In);
        assert_eq!(rules(&[sstl.clone(), ball.clone()]), ["BIVRU-1"]);
        let vref = [("35".to_owned(), 750, span())];
        assert!(check_pads(&[sstl.clone(), ball.clone()], &vref).is_empty());
        // Another bank's reference says nothing of this one.
        let other = [("34".to_owned(), 750, span())];
        assert_eq!(check_pads(&[sstl, ball.clone()], &other).len(), 1);
        // An output needs no reference.
        assert!(rules(&[pad("a", "35", "SSTL15", Out), ball]).is_empty());
        let bad = [("35".to_owned(), 500, span())];
        assert_eq!(check_pads(&[], &bad)[0].rule, "IVREF-2");
    }
}
