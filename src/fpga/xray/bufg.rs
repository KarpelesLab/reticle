//! The global clock buffer as more than a `BUFG`: a `BUFGCE` and a
//! `BUFGMUX` on the same `BUFGCTRL` site, configured per cell.
//!
//! # What a `BUFGCTRL` is
//!
//! Every global buffer of the 7 series is a `BUFGCTRL` (UG472): two clock
//! inputs `I0`/`I1`, each with a select `S` and an enable `CE`, and an
//! `IGNORE` per input that lets the output switch away from a clock that
//! has stopped. The library's simpler buffers are that one site with some
//! pins held:
//!
//! | Primitive | `I0` | `I1` | `CE0` | `CE1` | `S0` | `S1` | `IGNORE0` | `IGNORE1` |
//! |---|---|---|---|---|---|---|---|---|
//! | `BUFG` | `I` | — | 1 | 0 | 1 | 0 | 0 | 1 |
//! | `BUFGCE` | `I` | — | `CE` | 0 | 1 | 0 | 0 | 1 |
//! | `BUFGMUX` | `I0` | `I1` | `!S` | `S` | 1 | 1 | `ASYNC` | `ASYNC` |
//! | `BUFMRCE` | `I` | — | `CE` | 0 | 1 | 0 | 0 | 1 |
//! | `BUFR`, not dividing | `I` | — | 1 | 0 | 1 | 0 | 0 | 1 |
//!
//! The last two are **not** what Vivado does with them: they are regional
//! buffers, and here they are global ones. A `BUFMRCE` driving a `BUFR`
//! that does not divide is a gated clock, which is what the Husky's logic
//! analyser uses the pair for, and a `BUFGCE` followed by a `BUFG` is the
//! same function on the global network, at the cost of two of the part's
//! 32 global buffers instead of two regional ones. A `BUFR` that divides,
//! or whose enable or clear is used, is refused.
//!
//! A `BUFGMUX` selects with the **enables**, not the selects, because an
//! enable is what the site switches synchronously on (UG472, "BUFGMUX");
//! `IGNORE` is one only when `CLK_SEL_TYPE = "ASYNC"` asks it to switch
//! without waiting for the clock being left.
//!
//! # How a pin's value reaches the site
//!
//! The six controls arrive from interconnect, and an interconnect input
//! nothing drives **reads one** on this fabric (`super::cmt` has the
//! story). Each control has an inverter, and the database names its bit
//! the way it names a slice's `ZINI`: `ZINV_CE0` *set* means the inverter
//! is **off**. The two `IGNORE` bits are spelt `IS_IGNORE<n>_INVERTED` and
//! mean the same thing — set is *not* inverted. So:
//!
//! - a control tied to one is left unrouted with its bit set;
//! - a control tied to zero is left unrouted with its bit clear;
//! - a routed control has its bit set, unless the primitive wants it
//!   inverted (a `BUFGMUX`'s `CE0`, which is `!S`), when it is clear.
//!
//! # What checks it
//!
//! **Vivado's own configuration of thirteen buffers** in NewAE's shipped
//! ChipWhisperer-Husky bitstream, decoded through this database. The
//! three configurations it holds are exactly these, with the counts its
//! source predicts:
//!
//! ```text
//! 3  IN_USE IS_IGNORE1_INVERTED ZINV_CE0 ZINV_S0                            BUFG, BUFGCE
//! 8  IN_USE IS_IGNORE0_INVERTED IS_IGNORE1_INVERTED ZINV_CE1 ZINV_S0 ZINV_S1  BUFGMUX, ASYNC
//! 2  IN_USE ZINV_CE1 ZINV_S0 ZINV_S1                                       BUFGMUX, SYNC
//! ```
//!
//! — eight `BUFGMUX`es in its source say `CLK_SEL_TYPE("ASYNC")` and two
//! leave it at its `SYNC` default, which is what fixed the reading of the
//! `IGNORE` bits: the other way round, the counts would be swapped. What
//! the image does not show is which interconnect wire reached which
//! control, because the controls are reached through `IMUX` pips of the
//! interconnect tile and not through the buffer tile's own. That `S` goes
//! to *both* enables is UG472's, and the inverter on `CE0` alone is what
//! the image's bits require of it.
//!
//! `INIT_OUT`, `PRESELECT_I1` and `ZPRESELECT_I0` are clear in every one
//! of Vivado's thirteen, and are left clear here.

use super::XrayFabric;
use crate::fpga::arch::{ConfigEntry, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, BitstreamError};
use crate::fpga::place::{Netlist, Placement};
use crate::ir::{AttrValue, Design, ModuleId};
use crate::logic::Bit;

/// The `BUFGCTRL` features this module decides, each tagged with a
/// leading `=` so that no primitive is ever named after one: a
/// [`ConfigEntry::Cell`] fires for the primitive of its name, and these
/// must fire only when [`XrayFabric::configure_global_buffers`] says so.
pub(super) const FEATURES: [&str; 7] = [
    "=IN_USE",
    "=ZINV_CE0",
    "=ZINV_CE1",
    "=ZINV_S0",
    "=ZINV_S1",
    "=IS_IGNORE0_INVERTED",
    "=IS_IGNORE1_INVERTED",
];

/// The primitives configured here. A `BUFG` keeps the four fixed
/// features its bel declares for it, which every path through this flow
/// writes, so a design with only `BUFG`s is unchanged by this pass.
const CONFIGURED: [&str; 4] = ["BUFGCE", "BUFGMUX", "BUFMRCE", "BUFR"];

/// What one control pin of a placed buffer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    /// A signal routed to it.
    Routed,
    /// Tied to a constant, or left out, which reads as the value given.
    Fixed(bool),
}

/// Why a global buffer could not be configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlobalBufferError {
    /// A parameter this module reads holds something it does not take.
    BadParameter {
        /// The instance.
        instance: String,
        /// The parameter.
        name: String,
        /// Its value.
        value: String,
    },
    /// The database has no feature this needs.
    MissingFeature {
        /// The feature.
        feature: String,
    },
    /// A bit fell outside its tile.
    Bitstream(BitstreamError),
}

impl std::fmt::Display for GlobalBufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GlobalBufferError::BadParameter {
                instance,
                name,
                value,
            } => write!(
                f,
                "`{instance}`: `{name}` is `{value}`, which this flow does not configure"
            ),
            GlobalBufferError::MissingFeature { feature } => write!(
                f,
                "the chip database has no `BUFGCTRL` feature `{feature}`; the database is not the \
                 one this flow was written against"
            ),
            GlobalBufferError::Bitstream(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GlobalBufferError {}

impl From<BitstreamError> for GlobalBufferError {
    fn from(e: BitstreamError) -> Self {
        GlobalBufferError::Bitstream(e)
    }
}

/// The features a `BUFGCTRL` needs for the controls it is given, as
/// [`FEATURES`] names without the tag.
///
/// `controls` is `[CE0, CE1, S0, S1, IGNORE0, IGNORE1]`, each with whether
/// the primitive wants a routed signal inverted on the way in.
fn features(controls: [(Control, bool); 6]) -> Vec<&'static str> {
    const BITS: [&str; 6] = [
        "ZINV_CE0",
        "ZINV_CE1",
        "ZINV_S0",
        "ZINV_S1",
        "IS_IGNORE0_INVERTED",
        "IS_IGNORE1_INVERTED",
    ];
    let mut out = vec!["IN_USE"];
    for (bit, (control, invert)) in BITS.iter().zip(controls) {
        // Set means the inverter is off. An unrouted pin reads one, so a
        // fixed zero needs the inverter on and a fixed one needs it off.
        let inverter_off = match control {
            Control::Routed => !invert,
            Control::Fixed(value) => value,
        };
        if inverter_off {
            out.push(bit);
        }
    }
    out
}

impl XrayFabric {
    /// Configures every `BUFGCE` and `BUFGMUX` the placement put on a
    /// `BUFGCTRL`, and says how many bits that set.
    ///
    /// A pass of its own, after [`crate::fpga::bitstream::generate`], for
    /// the reason [`XrayFabric::configure_clock_managers`] is: what the
    /// site's bits are depends on whether each control was *routed*, and
    /// on a string parameter (`CLK_SEL_TYPE`) a
    /// [`ConfigEntry::Param`] cannot read.
    ///
    /// # Errors
    ///
    /// [`GlobalBufferError`]: a `CLK_SEL_TYPE` other than `SYNC` or
    /// `ASYNC`, or a database without the features.
    pub fn configure_global_buffers(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
        bits: &mut Bitstream,
    ) -> Result<usize, GlobalBufferError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(0);
        };
        let mut count = 0usize;
        for (index, instance) in netlist.instances.iter().enumerate() {
            if !CONFIGURED.contains(&instance.primitive.as_str()) {
                continue;
            }
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(bel) = self.arch.tile_types[site.tile_type].bel(&site.bel) else {
                continue;
            };
            let control = |role: &str, absent: bool| match instance
                .pins
                .iter()
                .map(|p| &netlist.pins[*p])
                .find(|p| p.role == role)
            {
                Some(p) if p.signal.is_some() => Control::Routed,
                Some(p) => Control::Fixed(p.constant != Some(Bit::Zero)),
                None => Control::Fixed(absent),
            };
            let fixed = |value: bool| (Control::Fixed(value), false);
            let controls = if instance.primitive == "BUFGMUX" {
                let params = m.cells.get(instance.cell).map(|cell| &cell.params);
                let select = params
                    .and_then(|p| p.get("CLK_SEL_TYPE"))
                    .and_then(AttrValue::as_str)
                    .unwrap_or("SYNC")
                    .to_ascii_uppercase();
                let ignore = match select.as_str() {
                    "SYNC" => false,
                    "ASYNC" => true,
                    _ => {
                        return Err(GlobalBufferError::BadParameter {
                            instance: instance.name.clone(),
                            name: "CLK_SEL_TYPE".to_owned(),
                            value: select,
                        });
                    }
                };
                // `S` is both enables, inverted into `CE0`. A select tied
                // to a constant is a constant on both, the first one
                // inverted by hand.
                let s = control("ce", false);
                let ce0 = match s {
                    Control::Routed => (Control::Routed, true),
                    Control::Fixed(v) => (Control::Fixed(!v), false),
                };
                [
                    ce0,
                    (control("ce1", false), false),
                    fixed(true),
                    fixed(true),
                    fixed(ignore),
                    fixed(ignore),
                ]
            } else {
                // A regional buffer pair the design asks for, built on the
                // global network instead: a `BUFR` that does not divide, is
                // always enabled and never cleared is a buffer and nothing
                // else, and a `BUFMRCE` in front of it is a gated one —
                // so the first is a `BUFG` and the second a `BUFGCE`.
                // Anything a `BUFR` does beyond that is refused.
                if instance.primitive == "BUFR" {
                    let params = m.cells.get(instance.cell).map(|cell| &cell.params);
                    let divide = params
                        .and_then(|p| p.get("BUFR_DIVIDE"))
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .or_else(|| v.as_int().map(|i| i.to_string()))
                                .unwrap_or_default()
                        })
                        .unwrap_or_else(|| "BYPASS".to_owned());
                    if !matches!(divide.as_str(), "1" | "BYPASS") {
                        return Err(GlobalBufferError::BadParameter {
                            instance: instance.name.clone(),
                            name: "BUFR_DIVIDE".to_owned(),
                            value: divide,
                        });
                    }
                    for (role, wanted) in [("ce", true), ("clr", false)] {
                        if control(role, wanted) != Control::Fixed(wanted) {
                            return Err(GlobalBufferError::BadParameter {
                                instance: instance.name.clone(),
                                name: role.to_ascii_uppercase(),
                                value: "a signal or the other constant".to_owned(),
                            });
                        }
                    }
                }
                // `BUFGCE`: a `BUFG` whose `CE0` is the enable.
                [
                    (control("ce", true), false),
                    fixed(false),
                    fixed(true),
                    fixed(false),
                    fixed(false),
                    fixed(true),
                ]
            };
            for name in features(controls) {
                let tag = format!("={name}");
                let entry = bel.config.iter().find_map(|entry| match entry {
                    ConfigEntry::Cell { primitive, bits } if *primitive == tag => Some(bits),
                    _ => None,
                });
                let Some(ones) = entry else {
                    return Err(GlobalBufferError::MissingFeature {
                        feature: name.to_owned(),
                    });
                };
                for bit in ones {
                    bits.set(site.tile, *bit)?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three configurations Vivado wrote for the Husky, from the
    /// three ways the table in the module documentation fills the pins.
    /// Would catch an inverter bit read the wrong way round (every
    /// configuration changes), `S` given to `CE0` uninverted, and the
    /// `IGNORE` pair tied to the wrong `CLK_SEL_TYPE`; would not catch a
    /// wrong reading shared by this test and the image, which is why the
    /// image's own counts are quoted above.
    #[test]
    fn the_three_configurations_vivado_wrote() {
        let fixed = |v| (Control::Fixed(v), false);
        let bufg = [
            fixed(true),
            fixed(false),
            fixed(true),
            fixed(false),
            fixed(false),
            fixed(true),
        ];
        assert_eq!(
            features(bufg),
            ["IN_USE", "ZINV_CE0", "ZINV_S0", "IS_IGNORE1_INVERTED"]
        );
        let bufgce = [
            (Control::Routed, false),
            fixed(false),
            fixed(true),
            fixed(false),
            fixed(false),
            fixed(true),
        ];
        assert_eq!(features(bufgce), features(bufg));
        let mux = |ignore| {
            [
                (Control::Routed, true),
                (Control::Routed, false),
                fixed(true),
                fixed(true),
                fixed(ignore),
                fixed(ignore),
            ]
        };
        assert_eq!(
            features(mux(true)),
            [
                "IN_USE",
                "ZINV_CE1",
                "ZINV_S0",
                "ZINV_S1",
                "IS_IGNORE0_INVERTED",
                "IS_IGNORE1_INVERTED"
            ]
        );
        assert_eq!(
            features(mux(false)),
            ["IN_USE", "ZINV_CE1", "ZINV_S0", "ZINV_S1"]
        );
    }
}
