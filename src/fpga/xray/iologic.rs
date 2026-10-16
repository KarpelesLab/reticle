//! The register beside a pad: an `IDDR` on an `ILOGICE3`, an `ODDR` on an
//! `OLOGICE3`, instantiated by name.
//!
//! # Where they sit, and how they are reached
//!
//! Every IO tile pair has, per pad, an `ILOGIC` between the pad's input
//! buffer and the interconnect and an `OLOGIC` between the interconnect and
//! the pad's output buffer. Until NewAE's ChipWhisperer-Husky this flow used
//! them only as wires ([`super::sites::pass_throughs`]); the Husky samples
//! its ADC with ten `IDDR`s and forwards clocks with five `ODDR`s, so they
//! are bels now too, of kind `ddr_in` and `ddr_out`.
//!
//! An `IDDR`'s `D` is `<side>_ILOGIC<n>_D`, which `ppips_lioi3.db` reaches
//! only from the pad (`LIOI_ILOGIC0_D ← LIOI_I0 ← LIOI_IBUF0`), and an
//! `ODDR`'s `Q` is `<side>_OLOGIC<n>_OQ`, which reaches only the pad. So the
//! placer puts each beside the buffer the constraints fix
//! (`place::follow_fixed_ends`). The clock reaches `CLK` — and, for an
//! `IDDR`, `CLKB` as well, which is what Vivado routes — from the IO
//! column's leaf clocks; the controls arrive from interconnect.
//!
//! # The bits, and where each came from
//!
//! **Vivado**, from NewAE's shipped Husky image decoded through this
//! database, where each of the Husky's `IDDR`s and `ODDR`s sits beside the
//! pad its source and `cwhusky.xdc` put it on:
//!
//! | Primitive and setting | Features (`ILOGIC_Y<n>.` / `OLOGIC_Y<n>.`) |
//! |---|---|
//! | `IDDR`, always | `IDDR.IN_USE`, `IDDR_OR_ISERDES.IN_USE`, `IFF.ZINV_C`, `ZINV_D` |
//! | `DDR_CLK_EDGE("OPPOSITE_EDGE")` | `IFF.DDR_CLK_EDGE.OPPOSITE_EDGE` |
//! | `SRTYPE("SYNC")` | `IFF.SRTYPE.SYNC` |
//! | `INIT_Q1(0)`, `INIT_Q2(0)` | `IFF.ZINIT_Q1`, `IFF.ZINIT_Q2` |
//! | `ODDR`, always | `ODDR_TDDR.IN_USE`, `OQUSED`, `OSERDES.DATA_RATE_OQ.DDR`, `OSERDES.DATA_RATE_TQ.BUF`, `ZINV_CLK` |
//! | `INIT(0)` | `ZINIT_OQ` |
//! | `SRTYPE("SYNC")` | `OSERDES.SRTYPE.SYNC`; `ASYNC` sets nothing |
//! | `D2` tied to 0 | `IS_D2_INVERTED`; `D1` tied to 1 sets nothing |
//! | `R` routed | `ODDR.SRUSED`, `ZSRVAL_OQ` |
//!
//! — one `IDDR` setting over ten instances and three `ODDR` settings over
//! three, every one with `D1` one and `D2` zero, two with a routed reset.
//!
//! The control inputs follow the rule the rest of this backend follows: a
//! pin left unrouted reads one, so a data input tied to zero is left
//! unrouted with its inverter on — which is what `IS_D2_INVERTED` is, and
//! unlike `ZINV_*`, the database names that one the right way round. What
//! the image cannot say, because no Husky instance does it, and this
//! refuses rather than guesses: a clock edge other than those two, a set or
//! reset tied to one, an enable tied to zero, an inverted clock or data
//! input, and an `IDDR` whose set or reset is routed.

use super::XrayFabric;
use super::sites::{BelPin, ExtraBel};
use crate::fpga::arch::{ConfigEntry, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, BitstreamError};
use crate::fpga::place::{Netlist, Placement};
use crate::ir::{AttrValue, Design, ModuleId};
use crate::logic::Bit;

/// The `ILOGIC` features [`XrayFabric::configure_io_logic`] may set, tagged
/// with a leading `=` as `super::bufg` tags its own.
const ILOGIC_FEATURES: [&str; 11] = [
    "=IDDR.IN_USE",
    "=IDDR_OR_ISERDES.IN_USE",
    "=IFF.DDR_CLK_EDGE.OPPOSITE_EDGE",
    "=IFF.DDR_CLK_EDGE.SAME_EDGE",
    "=IFF.SRTYPE.SYNC",
    "=IFF.ZINIT_Q1",
    "=IFF.ZINIT_Q2",
    "=IFF.ZSRVAL_Q1",
    "=IFF.ZSRVAL_Q2",
    "=IFF.ZINV_C",
    "=ZINV_D",
];

/// The `OLOGIC` features it may set.
const OLOGIC_FEATURES: [&str; 12] = [
    "=ODDR_TDDR.IN_USE",
    "=OQUSED",
    "=OSERDES.DATA_RATE_OQ.DDR",
    "=OSERDES.DATA_RATE_TQ.BUF",
    "=OSERDES.SRTYPE.SYNC",
    "=ODDR.DDR_CLK_EDGE.SAME_EDGE",
    "=ZINIT_OQ",
    "=ZINV_CLK",
    "=IS_D1_INVERTED",
    "=IS_D2_INVERTED",
    "=ODDR.SRUSED",
    "=ZSRVAL_OQ",
];

/// The `ILOGIC` and `OLOGIC` bels of an IO logic tile type, or nothing.
///
/// The wire numbering follows [`super::sites::pass_throughs`]: half `n`'s
/// wires carry the number `n`, except in a `_SING` tile, whose one half's
/// wires are all numbered 0.
pub(super) fn io_logic_bels(tile_type: &str) -> Vec<ExtraBel> {
    let (side, single) = match tile_type {
        "LIOI3" | "LIOI3_TBYTESRC" | "LIOI3_TBYTETERM" => ("LIOI", false),
        "RIOI3" | "RIOI3_TBYTESRC" | "RIOI3_TBYTETERM" => ("RIOI", false),
        "LIOI3_SING" => ("LIOI", true),
        "RIOI3_SING" => ("RIOI", true),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for n in 0..2u32 {
        let w = if single { 0 } else { n };
        let pin = |role: &'static str, wire: String| BelPin { role, wire };
        let tagged = |prefix: &str, names: &[&'static str]| -> Vec<(&'static str, Vec<String>)> {
            names
                .iter()
                .map(|name| (*name, vec![format!("{prefix}.{}", &name[1..])]))
                .collect()
        };
        out.push(ExtraBel {
            name: format!("ILOGIC_Y{n}"),
            kind: "ddr_in",
            pins: vec![
                pin("d", format!("{side}_ILOGIC{w}_D")),
                pin("clk", format!("IOI_ILOGIC{w}_CLK")),
                pin("clkb", format!("IOI_ILOGIC{w}_CLKB")),
                pin("ce", format!("IOI_ILOGIC{w}_CE1")),
                pin("sr", format!("IOI_ILOGIC{w}_SR")),
                pin("q1", format!("IOI_ILOGIC{w}_Q1")),
                pin("q2", format!("IOI_ILOGIC{w}_Q2")),
            ],
            cells: tagged(&format!("ILOGIC_Y{n}"), &ILOGIC_FEATURES),
        });
        out.push(ExtraBel {
            name: format!("OLOGIC_Y{n}"),
            kind: "ddr_out",
            pins: vec![
                pin("d1", format!("IOI_OLOGIC{w}_D1")),
                pin("d2", format!("IOI_OLOGIC{w}_D2")),
                pin("clk", format!("IOI_OLOGIC{w}_CLK")),
                pin("ce", format!("IOI_OLOGIC{w}_OCE")),
                pin("sr", format!("IOI_OLOGIC{w}_SR")),
                pin("q", format!("{side}_OLOGIC{w}_OQ")),
            ],
            cells: tagged(&format!("OLOGIC_Y{n}"), &OLOGIC_FEATURES),
        });
    }
    out
}

/// What a pin of a placed cell is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pin {
    Routed,
    Fixed(bool),
    Absent,
}

/// Why an IO register could not be configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IoLogicError {
    /// A setting no Husky instance has, which this refuses rather than
    /// guesses; see the module documentation.
    Unsupported {
        /// The instance.
        instance: String,
        /// What it asks for.
        message: String,
    },
    /// The database has no feature this needs.
    MissingFeature {
        /// The feature.
        feature: String,
    },
    /// A bit fell outside its tile.
    Bitstream(BitstreamError),
}

impl std::fmt::Display for IoLogicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoLogicError::Unsupported { instance, message } => write!(
                f,
                "`{instance}`: {message}, which this flow has no reading of the bits for"
            ),
            IoLogicError::MissingFeature { feature } => write!(
                f,
                "the chip database has no `{feature}`; the database is not the one this flow \
                 was written against"
            ),
            IoLogicError::Bitstream(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for IoLogicError {}

impl From<BitstreamError> for IoLogicError {
    fn from(e: BitstreamError) -> Self {
        IoLogicError::Bitstream(e)
    }
}

/// The features an `IDDR` needs, from its parameters and its pins.
fn iddr_features(
    text: &dyn Fn(&str, &str) -> String,
    int: &dyn Fn(&str, i64) -> i64,
    pin: &dyn Fn(&str) -> Pin,
) -> Result<Vec<&'static str>, String> {
    let mut out = vec![
        "IDDR.IN_USE",
        "IDDR_OR_ISERDES.IN_USE",
        "IFF.ZINV_C",
        "ZINV_D",
    ];
    match text("DDR_CLK_EDGE", "OPPOSITE_EDGE").as_str() {
        "OPPOSITE_EDGE" => out.push("IFF.DDR_CLK_EDGE.OPPOSITE_EDGE"),
        "SAME_EDGE" => out.push("IFF.DDR_CLK_EDGE.SAME_EDGE"),
        other => return Err(format!("`DDR_CLK_EDGE(\"{other}\")`")),
    }
    match text("SRTYPE", "SYNC").as_str() {
        "SYNC" => out.push("IFF.SRTYPE.SYNC"),
        "ASYNC" => {}
        other => return Err(format!("`SRTYPE(\"{other}\")`")),
    }
    if int("INIT_Q1", 0) == 0 {
        out.push("IFF.ZINIT_Q1");
    }
    if int("INIT_Q2", 0) == 0 {
        out.push("IFF.ZINIT_Q2");
    }
    if int("IS_C_INVERTED", 0) != 0 || int("IS_D_INVERTED", 0) != 0 {
        return Err("an inverted clock or data input".to_owned());
    }
    match pin("ce") {
        Pin::Fixed(false) => return Err("a clock enable tied to zero".to_owned()),
        Pin::Routed | Pin::Fixed(true) | Pin::Absent => {}
    }
    match pin("sr") {
        Pin::Routed => return Err("a routed set or reset".to_owned()),
        Pin::Fixed(true) => return Err("a set or reset tied to one".to_owned()),
        Pin::Fixed(false) | Pin::Absent => {}
    }
    Ok(out)
}

/// The features an `ODDR` needs. `reset` says which of `R` and `S` is the
/// routed one, if either is.
fn oddr_features(
    text: &dyn Fn(&str, &str) -> String,
    int: &dyn Fn(&str, i64) -> i64,
    pin: &dyn Fn(&str) -> Pin,
    reset: Option<bool>,
) -> Result<Vec<&'static str>, String> {
    let mut out = vec![
        "ODDR_TDDR.IN_USE",
        "OQUSED",
        "OSERDES.DATA_RATE_OQ.DDR",
        "OSERDES.DATA_RATE_TQ.BUF",
        "ZINV_CLK",
    ];
    match text("DDR_CLK_EDGE", "OPPOSITE_EDGE").as_str() {
        "OPPOSITE_EDGE" => {}
        "SAME_EDGE" => out.push("ODDR.DDR_CLK_EDGE.SAME_EDGE"),
        other => return Err(format!("`DDR_CLK_EDGE(\"{other}\")`")),
    }
    match text("SRTYPE", "SYNC").as_str() {
        "SYNC" => out.push("OSERDES.SRTYPE.SYNC"),
        "ASYNC" => {}
        other => return Err(format!("`SRTYPE(\"{other}\")`")),
    }
    if int("INIT", 0) == 0 {
        out.push("ZINIT_OQ");
    }
    if int("IS_C_INVERTED", 0) != 0 {
        return Err("an inverted clock".to_owned());
    }
    for (role, inverted) in [("d1", "IS_D1_INVERTED"), ("d2", "IS_D2_INVERTED")] {
        let param = int(inverted, 0) != 0;
        // An unrouted data pin reads one: a zero is the inverter on.
        let on = match pin(role) {
            Pin::Routed => param,
            Pin::Fixed(value) => !value,
            Pin::Absent => true,
        };
        if on {
            out.push(inverted);
        }
    }
    match pin("ce") {
        Pin::Fixed(false) => return Err("a clock enable tied to zero".to_owned()),
        Pin::Routed | Pin::Fixed(true) | Pin::Absent => {}
    }
    match reset {
        // `R` routed: the reset value is zero, which the `Z` spelling sets.
        Some(true) => out.extend(["ODDR.SRUSED", "ZSRVAL_OQ"]),
        Some(false) => out.push("ODDR.SRUSED"),
        None => {}
    }
    Ok(out)
}

impl XrayFabric {
    /// Configures every `IDDR` and `ODDR` the placement put on an IO logic
    /// site, and says how many bits that set.
    ///
    /// A pass of its own for the reason [`XrayFabric::configure_global_buffers`]
    /// is: a data input's inverter depends on whether it was routed, and
    /// the clock edge and reset type are string parameters.
    ///
    /// # Errors
    ///
    /// [`IoLogicError`]: a setting the module documentation lists as
    /// refused, or a database without the features.
    pub fn configure_io_logic(
        &self,
        design: &Design,
        module: ModuleId,
        graph: &RoutingGraph,
        netlist: &Netlist,
        placement: &Placement,
        bits: &mut Bitstream,
    ) -> Result<usize, IoLogicError> {
        let Some(m) = design.modules.get(module) else {
            return Ok(0);
        };
        let mut count = 0usize;
        for (index, instance) in netlist.instances.iter().enumerate() {
            let input = match instance.primitive.as_str() {
                "IDDR" => true,
                "ODDR" => false,
                _ => continue,
            };
            let Some(site) = placement.site_of(index) else {
                continue;
            };
            let site = &graph.sites[site];
            let Some(bel) = self.arch.tile_types[site.tile_type].bel(&site.bel) else {
                continue;
            };
            let params = m.cells.get(instance.cell).map(|cell| &cell.params);
            let text = |name: &str, default: &str| {
                params
                    .and_then(|p| p.get(name))
                    .and_then(AttrValue::as_str)
                    .unwrap_or(default)
                    .to_ascii_uppercase()
            };
            let int = |name: &str, default: i64| {
                params
                    .and_then(|p| p.get(name))
                    .and_then(AttrValue::as_int)
                    .unwrap_or(default)
            };
            let pins: Vec<&crate::fpga::place::NetPin> =
                instance.pins.iter().map(|p| &netlist.pins[*p]).collect();
            let state = |role: &str| -> Pin {
                let mut found = Pin::Absent;
                for p in pins.iter().filter(|p| p.role == role) {
                    match (p.signal, p.constant) {
                        (Some(_), _) => return Pin::Routed,
                        (None, Some(Bit::One)) => found = Pin::Fixed(true),
                        (None, _) => {
                            if found == Pin::Absent {
                                found = Pin::Fixed(false);
                            }
                        }
                    }
                }
                found
            };
            let names = if input {
                iddr_features(&text, &int, &state)
            } else {
                // Which of the two ports on the one `SR` pin is routed.
                let routed = |port: &str| {
                    pins.iter()
                        .any(|p| p.role == "sr" && p.port == port && p.signal.is_some())
                };
                let reset = match (routed("R"), routed("S")) {
                    (true, false) => Some(true),
                    (false, true) => Some(false),
                    (false, false) => None,
                    (true, true) => {
                        return Err(IoLogicError::Unsupported {
                            instance: instance.name.clone(),
                            message: "both a set and a reset routed".to_owned(),
                        });
                    }
                };
                oddr_features(&text, &int, &state, reset)
            }
            .map_err(|message| IoLogicError::Unsupported {
                instance: instance.name.clone(),
                message,
            })?;
            for name in names {
                let tag = format!("={name}");
                let entry = bel.config.iter().find_map(|entry| match entry {
                    ConfigEntry::Cell { primitive, bits } if *primitive == tag => Some(bits),
                    _ => None,
                });
                let Some(ones) = entry else {
                    return Err(IoLogicError::MissingFeature {
                        feature: format!("{}.{name}", site.bel),
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

    /// The Husky's settings, from the module documentation's table, worked
    /// through the two functions. Would catch an `INIT` read the wrong way
    /// round, a zero data input left without its inverter, `SRTYPE("ASYNC")`
    /// written as `SYNC`, and a routed reset given the set's value; would not
    /// catch a wrong reading shared by this test and the image, which is why
    /// the table is Vivado's.
    #[test]
    fn the_settings_vivado_wrote_for_the_husky() {
        let text = |name: &str, default: &str| match name {
            "SRTYPE" => "SYNC".to_owned(),
            _ => default.to_owned(),
        };
        let int = |_: &str, default: i64| default;
        let tied = |role: &str| match role {
            "d1" => Pin::Fixed(true),
            "d2" => Pin::Fixed(false),
            "ce" => Pin::Fixed(true),
            _ => Pin::Fixed(false),
        };
        let mut iddr = iddr_features(&text, &int, &tied).unwrap();
        iddr.sort_unstable();
        assert_eq!(
            iddr,
            [
                "IDDR.IN_USE",
                "IDDR_OR_ISERDES.IN_USE",
                "IFF.DDR_CLK_EDGE.OPPOSITE_EDGE",
                "IFF.SRTYPE.SYNC",
                "IFF.ZINIT_Q1",
                "IFF.ZINIT_Q2",
                "IFF.ZINV_C",
                "ZINV_D"
            ]
        );
        let mut oddr = oddr_features(&text, &int, &tied, None).unwrap();
        oddr.sort_unstable();
        assert_eq!(
            oddr,
            [
                "IS_D2_INVERTED",
                "ODDR_TDDR.IN_USE",
                "OQUSED",
                "OSERDES.DATA_RATE_OQ.DDR",
                "OSERDES.DATA_RATE_TQ.BUF",
                "OSERDES.SRTYPE.SYNC",
                "ZINIT_OQ",
                "ZINV_CLK"
            ]
        );
        // The glitch outputs: `SRTYPE("ASYNC")`, reset routed.
        let asynchronous = |name: &str, default: &str| match name {
            "SRTYPE" => "ASYNC".to_owned(),
            _ => default.to_owned(),
        };
        let mut glitch = oddr_features(&asynchronous, &int, &tied, Some(true)).unwrap();
        glitch.sort_unstable();
        assert_eq!(
            glitch,
            [
                "IS_D2_INVERTED",
                "ODDR.SRUSED",
                "ODDR_TDDR.IN_USE",
                "OQUSED",
                "OSERDES.DATA_RATE_OQ.DDR",
                "OSERDES.DATA_RATE_TQ.BUF",
                "ZINIT_OQ",
                "ZINV_CLK",
                "ZSRVAL_OQ"
            ]
        );
        // And what is refused.
        let routed_reset = |role: &str| {
            if role == "sr" {
                Pin::Routed
            } else {
                tied(role)
            }
        };
        assert!(iddr_features(&text, &int, &routed_reset).is_err());
    }
}
