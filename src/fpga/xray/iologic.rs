//! The register beside a pad: an `IDDR` or `IDDR_2CLK` on an `ILOGICE3`, an
//! `ODDR` on an `OLOGICE3`, instantiated by name; and the serialisers on the
//! same sites, an `ISERDESE2` and an `OSERDESE2`.
//!
//! # The serialisers
//!
//! **Vivado 2026.1, every setting separately**, since 2026-11-05: 192 of
//! them in `testdata/fpga/vivado/iologic/` (`iserdes_*`, `oserdes_*`,
//! `serdes_feedback`), every `ILOGIC` and `OLOGIC` feature, ground, tie and
//! clock source of each held to Vivado's by `tests/fpga_xray_ioreg.rs`, and
//! the twelve designs Vivado refuses refused by its rule's name. The features
//! of each setting are [`iserdes_features`]' and [`oserdes_features`]'
//! tables; the rules, and what the clock inversions write (which is not what
//! their names say), are `xc7::serdes`'s. Five settings set bits the database
//! does not name, which [`measured_features`] names. A width of ten or
//! fourteen is a master on half 0 and a slave on half 1, joined by the
//! tile's cascade wires, which `place::follow_fixed_ends` follows from the
//! master's pad.
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
//! (`place::follow_fixed_ends`). The clock reaches `CLK` and `CLKB` — both,
//! for an `IDDR` and an `ODDR` alike, which is what Vivado routes — from the IO
//! column's leaf clocks; the controls arrive from interconnect. An
//! `IDDR_2CLK`'s `CB` is `CLKB`, which inverts what it is given: so `~C` there
//! is `C` on the pin (`flow::primitive_instances`).
//!
//! # The bits, and where each came from
//!
//! **Vivado 2026.1, every setting separately**, since 2026-11-04:
//! `tools/vivado-parity/gen_iologic.py` puts one setting beside each pad of a
//! bank, and `tests/fpga_xray_ioreg.rs` holds every `ILOGIC` and `OLOGIC`
//! feature of all 58 to `testdata/fpga/vivado/iologic/vivado.txt`. What that
//! added to the table below, which it agreed with line by line:
//!
//! | Setting | Features |
//! |---|---|
//! | `IDDR` `SAME_EDGE` | `IFF.DDR_CLK_EDGE.SAME_EDGE`, and `ZINIT_Q3`/`ZINIT_Q4` following `INIT_Q1`/`INIT_Q2`, and `ZSRVAL_Q3`/`Q4` with a reset |
//! | `IDDR` `SAME_EDGE_PIPELINED` | the same, with no edge feature |
//! | `IDDR` `R` in use | `IFF.ZSRVAL_Q1`, `ZSRVAL_Q2`; `S` sets nothing |
//! | `IS_C_INVERTED`, `IS_D_INVERTED` | `IFF.ZINV_C`, `ZINV_D` clear |
//! | `ODDR` `IS_C_INVERTED` | `ZINV_CLK` clear, **and the edge feature turned round**: `SAME_EDGE` is set at `OPPOSITE_EDGE`, and clear at `SAME_EDGE` |
//! | a set or reset tied high | as routed, with the pin left unrouted, which reads a one |
//! | a clock enable tied low | the same site bits as tied high: the pin is grounded, by `bram::io_logic_wants` |
//! | `IDDR_2CLK` | `IDDR`'s; `CB` of `~C`, or `C` with one of `IS_C_INVERTED` and `IS_CB_INVERTED`, is `C` on `CLKB`, and a second clock is routed there |
//!
//! Refused, as Vivado refuses them: a set and a reset both in use (`REQP-77`,
//! `REQP-129`), and `C` on `CB` with one inversion (`REQP-76`). Not measured,
//! and refused: `IS_CB_INVERTED` on a second clock. The table below was the
//! first account:
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
//! unlike `ZINV_*`, the database names that one the right way round. This
//! refused, from the image alone, a clock edge other than those two, a set or
//! reset tied to one, an enable tied to zero, an inverted clock or data input,
//! and an `IDDR` whose set or reset is routed; Vivado's answers for each are
//! the table above, and now they are built.

use super::XrayFabric;
use super::sites::{BelPin, ExtraBel};
use crate::fpga::arch::{ConfigBit, ConfigEntry, RoutingGraph};
use crate::fpga::bitstream::{Bitstream, BitstreamError};
use crate::fpga::place::{Netlist, Placement};
use crate::fpga::xc7::serdes;
use crate::ir::{AttrValue, Design, ModuleId};
use crate::logic::Bit;

/// Bits of a tile, as `(frame, bit)`.
type Bits = &'static [(u32, u32)];

/// A serialiser's bits the database does not name, measured on Vivado's
/// (`?` lines of `reticle fpga --decode`), as `(name, ones, zeros)` on half 0,
/// `(frame, bit)` in the tile. **The names are mine.** Half 1's are the same
/// bits mirrored, frame `f ^ 1` and bit `127 - b`, as every `ILOGIC` and
/// `OLOGIC` feature of the database is; each was also seen there, a slave or
/// an `_N` ball, but `IS_CLKDIVP_INVERTED`, whose half 1 is the mirror alone.
/// The zeros of a width are the rest of the database's `DATA_WIDTH` field, and
/// the other width's new bit.
const MEASURED: [(&str, Bits, Bits); 6] = [
    // `iserdes_sweep`, `iserdes_pins`: `IS_CLKDIV_INVERTED`, and nothing else.
    ("ILOGIC_Y0.IS_CLKDIV_INVERTED", &[(26, 119)], &[]),
    // `iserdes_clkdivp`: `CLKDIVP` on `CLKDIV`'s clock, and inverted.
    ("ILOGIC_Y0.CLKDIVP_USED", &[(29, 98)], &[]),
    ("ILOGIC_Y0.IS_CLKDIVP_INVERTED", &[(27, 114)], &[]),
    // `oserdes_sweep`, `oserdes_tristate`: `TBYTE_CTL("TRUE")`.
    ("OLOGIC_Y0.OSERDES.TBYTE_CTL", &[(32, 80)], &[]),
    // `oserdes_wide`, on the master and on the slave.
    (
        "OLOGIC_Y0.OSERDES.DATA_WIDTH.DDR.W10",
        &[(30, 99), (30, 115), (31, 98), (33, 73)],
        &[
            (30, 95),
            (30, 101),
            (30, 121),
            (30, 123),
            (30, 127),
            (31, 100),
            (31, 116),
            (31, 120),
            (31, 124),
            (31, 126),
        ],
    ),
    (
        "OLOGIC_Y0.OSERDES.DATA_WIDTH.DDR.W14",
        &[(30, 101), (31, 98), (31, 100), (33, 73)],
        &[
            (30, 95),
            (30, 99),
            (30, 115),
            (30, 121),
            (30, 123),
            (30, 127),
            (31, 116),
            (31, 120),
            (31, 124),
            (31, 126),
        ],
    ),
];

/// [`MEASURED`] as features of an IO logic tile type, both halves, for the
/// database to hold beside its own (`XrayDatabase::features`). The six IO
/// logic tile types have the same bits for every `ILOGIC` and `OLOGIC` feature
/// the database names, so these, measured on `RIOI3` tiles, are given to all
/// of them: **that is the database's word for the `LIOI3` side, not a
/// measurement**.
pub(super) fn measured_features(tile_type: &str) -> Vec<super::Feature> {
    if !matches!(
        tile_type,
        "LIOI3"
            | "LIOI3_TBYTESRC"
            | "LIOI3_TBYTETERM"
            | "RIOI3"
            | "RIOI3_TBYTESRC"
            | "RIOI3_TBYTETERM"
    ) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (name, ones, zeros) in MEASURED {
        for half in 0..2u32 {
            let bits = |list: &[(u32, u32)]| {
                list.iter()
                    .map(|&(frame, bit)| {
                        if half == 0 {
                            ConfigBit::new(frame, bit)
                        } else {
                            ConfigBit::new(frame ^ 1, 127 - bit)
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let name = if half == 0 {
                name.to_owned()
            } else {
                name.replacen("_Y0.", "_Y1.", 1)
            };
            out.push(super::Feature {
                name,
                ones: bits(ones),
                zeros: bits(zeros),
            });
        }
    }
    out
}

/// What Vivado refuses of the fabric around a serialiser, before anything
/// is placed: `OFB` read by anything but the `ISERDESE2` beside it, or `TFB`
/// straight into a flip-flop that drives a pad (`THRU-2`, `oserdes_ofb_fabric`,
/// `oserdes_tfb_fabric`; `TFB` through a second flip-flop is built,
/// `oserdes_tfb_pipelined`).
///
/// # Errors
///
/// The refusal, by Vivado's rule.
pub fn check_serdes_wiring(netlist: &Netlist) -> Result<(), String> {
    for instance in netlist
        .instances
        .iter()
        .filter(|i| i.primitive == "OSERDESE2")
    {
        for p in instance.pins.iter().map(|p| &netlist.pins[*p]) {
            let Some(signal) = p.signal.filter(|_| p.output) else {
                continue;
            };
            let sinks = || {
                netlist.signals[signal].sinks.iter().map(|s| {
                    (
                        &netlist.pins[*s],
                        &netlist.instances[netlist.pins[*s].instance],
                    )
                })
            };
            let on_a_pad = |ff: &crate::fpga::place::Instance| {
                ff.pins
                    .iter()
                    .map(|q| &netlist.pins[*q])
                    .filter(|q| q.output)
                    .filter_map(|q| q.signal)
                    .flat_map(|s| netlist.signals[s].sinks.iter())
                    .any(|s| netlist.instances[netlist.pins[*s].instance].kind == "io")
            };
            let refused = match p.role.as_str() {
                "ofb" => {
                    sinks().any(|(pin, to)| !(to.primitive == "ISERDESE2" && pin.role == "ofb"))
                }
                "tfb" => sinks().any(|(_, to)| to.kind == "ff" && on_a_pad(to)),
                _ => false,
            };
            if refused {
                return Err(format!(
                    "`{}`: its `{}` is read by the fabric{}, which Vivado refuses (THRU-2)",
                    instance.name,
                    p.port,
                    if p.role == "ofb" {
                        "; it reaches only the `ISERDESE2` beside it"
                    } else {
                        " through a flip-flop on a pad"
                    }
                ));
            }
        }
    }
    Ok(())
}

/// The `ILOGIC` features [`XrayFabric::configure_io_logic`] may set, tagged
/// with a leading `=` as `super::bufg` tags its own.
const ILOGIC_FEATURES: [&str; 15] = [
    "=IDDR.IN_USE",
    "=IDDR_OR_ISERDES.IN_USE",
    "=IFF.DDR_CLK_EDGE.OPPOSITE_EDGE",
    "=IFF.DDR_CLK_EDGE.SAME_EDGE",
    "=IFF.SRTYPE.SYNC",
    "=IFF.ZINIT_Q1",
    "=IFF.ZINIT_Q2",
    "=IFF.ZINIT_Q3",
    "=IFF.ZINIT_Q4",
    "=IFF.ZSRVAL_Q1",
    "=IFF.ZSRVAL_Q2",
    "=IFF.ZSRVAL_Q3",
    "=IFF.ZSRVAL_Q4",
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

/// A serialiser's numbered pins' roles: `Q1` is `q1`, as an `IDDR`'s is.
const Q_ROLES: [&str; 8] = ["q1", "q2", "q3", "q4", "q5", "q6", "q7", "q8"];
/// And an `OSERDESE2`'s data inputs, `D1` being an `ODDR`'s `d1`.
const D_ROLES: [&str; 8] = ["d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8"];
/// And its tristate inputs.
const T_ROLES: [&str; 4] = ["t1", "t2", "t3", "t4"];

/// The `ILOGIC` features an `ISERDESE2` may set, beyond [`ILOGIC_FEATURES`].
/// One with a leading `=O:` is the same half's `OLOGIC`'s: an `ISERDESE2`
/// whose `OCLK` is in use sets `ODDR_TDDR.IN_USE` there.
const ISERDES_FEATURES: [&str; 31] = [
    "=ISERDES.IN_USE",
    "=ISERDES.NUM_CE.N2",
    "=ISERDES.MODE.SLAVE",
    "=ISERDES.OFB_USED",
    "=ISERDES.DYN_CLK_INV_EN",
    "=ISERDES.DYN_CLKDIV_INV_EN",
    "=IFF.ZINV_OCLK",
    "=IFF.INV_OCLK",
    "=IDELMUXE3.P0",
    "=IDELMUXE3.P1",
    "=IFFDELMUXE3.P0",
    "=IS_CLKDIV_INVERTED",
    "=CLKDIVP_USED",
    "=IS_CLKDIVP_INVERTED",
    "=ISERDES.NETWORKING.SDR.W2",
    "=ISERDES.NETWORKING.SDR.W3",
    "=ISERDES.NETWORKING.SDR.W4",
    "=ISERDES.NETWORKING.SDR.W5",
    "=ISERDES.NETWORKING.SDR.W6",
    "=ISERDES.NETWORKING.SDR.W7",
    "=ISERDES.NETWORKING.SDR.W8",
    "=ISERDES.NETWORKING.DDR.W4",
    "=ISERDES.NETWORKING.DDR.W6",
    "=ISERDES.NETWORKING.DDR.W8",
    "=ISERDES.NETWORKING.DDR.W10",
    "=ISERDES.NETWORKING.DDR.W14",
    "=ISERDES.MEMORY.DDR.W4",
    "=ISERDES.MEMORY_DDR3.DDR.W4",
    "=ISERDES.MEMORY_QDR.DDR.W4",
    "=ISERDES.OVERSAMPLE.DDR.W4",
    "=O:ODDR_TDDR.IN_USE",
];

/// The `OLOGIC` features an `OSERDESE2` may set, beyond [`OLOGIC_FEATURES`].
const OSERDES_FEATURES: [&str; 34] = [
    "=OSERDES.IN_USE",
    "=OSERDES.DATA_RATE_OQ.SDR",
    "=OSERDES.DATA_RATE_TQ.SDR",
    "=OSERDES.DATA_RATE_TQ.DDR",
    "=OSERDES.DATA_WIDTH.SDR.W2",
    "=OSERDES.DATA_WIDTH.SDR.W3",
    "=OSERDES.DATA_WIDTH.SDR.W4",
    "=OSERDES.DATA_WIDTH.SDR.W5",
    "=OSERDES.DATA_WIDTH.SDR.W6",
    "=OSERDES.DATA_WIDTH.SDR.W7",
    "=OSERDES.DATA_WIDTH.SDR.W8",
    "=OSERDES.DATA_WIDTH.DDR.W4",
    "=OSERDES.DATA_WIDTH.DDR.W6",
    "=OSERDES.DATA_WIDTH.DDR.W8",
    "=OSERDES.DATA_WIDTH.DDR.W10",
    "=OSERDES.DATA_WIDTH.DDR.W14",
    "=OSERDES.TRISTATE_WIDTH.W4",
    "=OSERDES.TSRTYPE.SYNC",
    "=OSERDES.SERDES_MODE.SLAVE",
    "=OSERDES.TBYTE_CTL",
    "=TDDR.SRUSED",
    "=ZINIT_TQ",
    "=ZSRVAL_TQ",
    "=IS_CLKDIV_INVERTED",
    "=IS_D3_INVERTED",
    "=IS_D4_INVERTED",
    "=IS_D5_INVERTED",
    "=IS_D6_INVERTED",
    "=IS_D7_INVERTED",
    "=IS_D8_INVERTED",
    "=ZINV_T1",
    "=ZINV_T2",
    "=ZINV_T3",
    "=ZINV_T4",
];

/// The `ILOGIC` and `OLOGIC` bels of an IO logic tile type, or nothing.
///
/// The wire numbering follows [`super::sites::pass_throughs`]: half `n`'s
/// wires carry the number `n`, except in a `_SING` tile, whose one half's
/// wires are all numbered 0.
///
/// A serialiser's pins are the same sites': an `ISERDESE2` is an `ILOGIC`,
/// as an `IDDR` is, and an `OSERDESE2` an `OLOGIC`. Its cascade is half 0's
/// `SHIFTOUT` into half 1's `SHIFTIN` for an `ISERDESE2` (`ISIN11 ← ISOUT10`
/// in `ppips_rioi3.db`), and the other way for an `OSERDESE2`
/// (`OSIN10 ← OSOUT11`), so a master is always on half 0, the pair's `_P`
/// ball. `OCLKB` and `CLKDIVP` have no pin here: nothing in the database
/// reaches them, and what Vivado writes does not depend on them
/// (`iserdes_clocks`); `xc7::serdes` drops them.
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
        let mut ipins = vec![
            pin("d", format!("{side}_ILOGIC{w}_D")),
            pin("clk", format!("IOI_ILOGIC{w}_CLK")),
            pin("clkb", format!("IOI_ILOGIC{w}_CLKB")),
            pin("ce", format!("IOI_ILOGIC{w}_CE1")),
            pin("sr", format!("IOI_ILOGIC{w}_SR")),
            pin("q1", format!("IOI_ILOGIC{w}_Q1")),
            pin("q2", format!("IOI_ILOGIC{w}_Q2")),
        ];
        // An `ISERDESE2`'s, beyond those.
        ipins.extend([
            pin("ddly", format!("{side}_ILOGIC{w}_DDLY")),
            pin("ofb", format!("{side}_ILOGIC{w}_OFB")),
            pin("clkdiv", format!("IOI_ILOGIC{w}_CLKDIV")),
            pin("oclk", format!("IOI_ILOGIC{w}_OCLK")),
            pin("ce2", format!("IOI_ILOGIC{w}_CE2")),
            pin("bitslip", format!("IOI_ILOGIC{w}_BITSLIP")),
            pin("dynclksel", format!("IOI_ILOGIC{w}_DYNCLKSEL")),
            pin("dynclkdivsel", format!("IOI_ILOGIC{w}_DYNCLKDIVSEL")),
            pin("o", format!("IOI_ILOGIC{w}_O")),
            pin("shiftout1", format!("{side}_ISOUT1{w}")),
            pin("shiftout2", format!("{side}_ISOUT2{w}")),
        ]);
        ipins.extend((3..=8).map(|q| pin(Q_ROLES[q - 1], format!("IOI_ILOGIC{w}_Q{q}"))));
        // Only half 1 has a cascade in (`ISIN11 ← ISOUT10`).
        if n == 1 && !single {
            ipins.push(pin("shiftin1", format!("{side}_ISIN11")));
            ipins.push(pin("shiftin2", format!("{side}_ISIN21")));
        }
        let mut icells = tagged(&format!("ILOGIC_Y{n}"), &ILOGIC_FEATURES);
        icells.extend(ISERDES_FEATURES.iter().map(|name| {
            let full = match name.strip_prefix("=O:") {
                Some(rest) => format!("OLOGIC_Y{n}.{rest}"),
                None => format!("ILOGIC_Y{n}.{}", &name[1..]),
            };
            (*name, vec![full])
        }));
        out.push(ExtraBel {
            name: format!("ILOGIC_Y{n}"),
            kind: "ddr_in",
            pins: ipins,
            cells: icells,
        });
        let mut opins = vec![
            pin("d1", format!("IOI_OLOGIC{w}_D1")),
            pin("d2", format!("IOI_OLOGIC{w}_D2")),
            pin("clk", format!("IOI_OLOGIC{w}_CLK")),
            // Vivado routes the clock here too (`oddr_sweep`, every one).
            pin("clkb", format!("IOI_OLOGIC{w}_CLKB")),
            pin("ce", format!("IOI_OLOGIC{w}_OCE")),
            pin("sr", format!("IOI_OLOGIC{w}_SR")),
            pin("q", format!("{side}_OLOGIC{w}_OQ")),
        ];
        // An `OSERDESE2`'s, beyond those.
        opins.extend((3..=8).map(|d| pin(D_ROLES[d - 1], format!("IOI_OLOGIC{w}_D{d}"))));
        opins.extend((1..=4).map(|t| pin(T_ROLES[t - 1], format!("IOI_OLOGIC{w}_T{t}"))));
        opins.extend([
            pin("clkdiv", format!("IOI_OLOGIC{w}_CLKDIV")),
            pin("tce", format!("IOI_OLOGIC{w}_TCE")),
            pin("tq", format!("{side}_OLOGIC{w}_TQ")),
            pin("ofb", format!("{side}_OLOGIC{w}_OFB")),
            pin("tfb", format!("{side}_OLOGIC{w}_TFB")),
            pin("shiftout1", format!("{side}_OSOUT1{w}")),
            pin("shiftout2", format!("{side}_OSOUT2{w}")),
        ]);
        // Only half 0 has a cascade in (`OSIN10 ← OSOUT11`).
        if n == 0 {
            opins.push(pin("shiftin1", format!("{side}_OSIN10")));
            opins.push(pin("shiftin2", format!("{side}_OSIN20")));
        }
        let mut ocells = tagged(&format!("OLOGIC_Y{n}"), &OLOGIC_FEATURES);
        ocells.extend(tagged(&format!("OLOGIC_Y{n}"), &OSERDES_FEATURES));
        out.push(ExtraBel {
            name: format!("OLOGIC_Y{n}"),
            kind: "ddr_out",
            pins: opins,
            cells: ocells,
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

/// The features an `IDDR` or `IDDR_2CLK` needs, from its parameters and its
/// pins. `reset` says which of `R` and `S` is in use, if either is: routed, or
/// tied high and left unrouted, which reads a one. **Every line is Vivado's**,
/// `testdata/fpga/vivado/iologic/`: the same-edge modes keep the second pair of
/// registers, `Q3` and `Q4`, with `INIT_Q1` and `INIT_Q2` and the reset's value;
/// `SAME_EDGE_PIPELINED` has no edge feature of its own; and an inversion clears
/// its `ZINV`.
fn iddr_features(
    text: &dyn Fn(&str, &str) -> String,
    int: &dyn Fn(&str, i64) -> i64,
    reset: Option<bool>,
) -> Result<Vec<&'static str>, String> {
    let mut out = vec!["IDDR.IN_USE", "IDDR_OR_ISERDES.IN_USE"];
    if int("IS_C_INVERTED", 0) == 0 {
        out.push("IFF.ZINV_C");
    }
    if int("IS_D_INVERTED", 0) == 0 {
        out.push("ZINV_D");
    }
    let pipelined = match text("DDR_CLK_EDGE", "OPPOSITE_EDGE").as_str() {
        "OPPOSITE_EDGE" => {
            out.push("IFF.DDR_CLK_EDGE.OPPOSITE_EDGE");
            false
        }
        "SAME_EDGE" => {
            out.push("IFF.DDR_CLK_EDGE.SAME_EDGE");
            true
        }
        "SAME_EDGE_PIPELINED" => true,
        other => return Err(format!("`DDR_CLK_EDGE(\"{other}\")`")),
    };
    match text("SRTYPE", "SYNC").as_str() {
        "SYNC" => out.push("IFF.SRTYPE.SYNC"),
        "ASYNC" => {}
        other => return Err(format!("`SRTYPE(\"{other}\")`")),
    }
    if int("INIT_Q1", 0) == 0 {
        out.push("IFF.ZINIT_Q1");
        if pipelined {
            out.push("IFF.ZINIT_Q3");
        }
    }
    if int("INIT_Q2", 0) == 0 {
        out.push("IFF.ZINIT_Q2");
        if pipelined {
            out.push("IFF.ZINIT_Q4");
        }
    }
    // `R`: the value it sets is zero, which the `Z` spelling says; `S` is the
    // field clear.
    if reset == Some(true) {
        out.extend(["IFF.ZSRVAL_Q1", "IFF.ZSRVAL_Q2"]);
        if pipelined {
            out.extend(["IFF.ZSRVAL_Q3", "IFF.ZSRVAL_Q4"]);
        }
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
    ];
    // An inverted clock clears `ZINV_CLK` and turns the edge feature round:
    // `oddr_sweep` has `SAME_EDGE` set for an inverted clock at
    // `OPPOSITE_EDGE`, and clear at `SAME_EDGE`. Measured; why is not known.
    let inverted = int("IS_C_INVERTED", 0) != 0;
    if !inverted {
        out.push("ZINV_CLK");
    }
    let same = match text("DDR_CLK_EDGE", "OPPOSITE_EDGE").as_str() {
        "OPPOSITE_EDGE" => false,
        "SAME_EDGE" => true,
        other => return Err(format!("`DDR_CLK_EDGE(\"{other}\")`")),
    };
    if same != inverted {
        out.push("ODDR.DDR_CLK_EDGE.SAME_EDGE");
    }
    match text("SRTYPE", "SYNC").as_str() {
        "SYNC" => out.push("OSERDES.SRTYPE.SYNC"),
        "ASYNC" => {}
        other => return Err(format!("`SRTYPE(\"{other}\")`")),
    }
    if int("INIT", 0) == 0 {
        out.push("ZINIT_OQ");
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
    match reset {
        // `R` routed: the reset value is zero, which the `Z` spelling sets.
        Some(true) => out.extend(["ODDR.SRUSED", "ZSRVAL_OQ"]),
        Some(false) => out.push("ODDR.SRUSED"),
        None => {}
    }
    Ok(out)
}

/// The `&'static` spelling of `name` among `list`'s (`=`-tagged) features.
fn named(list: &'static [&'static str], name: &str) -> Option<&'static str> {
    list.iter()
        .find(|tag| &tag[1..] == name)
        .map(|tag| &tag[1..])
}

/// The features an `ISERDESE2` needs. `zinv_c` is
/// [`serdes::Iserdes::clock_inverter`]'s answer. **Every line is Vivado's**,
/// `testdata/fpga/vivado/iologic/` (`iserdes_*`, `serdes_feedback`):
///
/// | Setting | Features |
/// |---|---|
/// | always | `IDDR_OR_ISERDES.IN_USE`, `ISERDES.IN_USE`, `IFF.DDR_CLK_EDGE.OPPOSITE_EDGE`, `IFF.SRTYPE.SYNC`; `IDDR.IN_USE` too in every interface but `MEMORY` |
/// | `INIT_Q<k>`, `SRVAL_Q<k>` 0 | `IFF.ZINIT_Q<k>`, `IFF.ZSRVAL_Q<k>`, whatever the reset |
/// | interface, rate, width | `ISERDES.<interface>.<rate>.W<width>` |
/// | `NUM_CE(2)`, `SLAVE`, `OFB_USED`, `DYN_*_INV_EN` | `NUM_CE.N2`, `MODE.SLAVE`, `OFB_USED`, `DYN_CLK_INV_EN`, `DYN_CLKDIV_INV_EN`; a master sets nothing |
/// | `D` routed, not inverted | `ZINV_D`; a slave's, or one reading `OFB`, nothing |
/// | `OCLK` routed | `IFF.ZINV_OCLK`, or `IFF.INV_OCLK` inverted, and the `OLOGIC`'s `ODDR_TDDR.IN_USE` |
/// | `IS_CLKDIV_INVERTED` | a bit the database does not name, `IS_CLKDIV_INVERTED` here ([`measured_features`]) |
/// | `IOBDELAY` `IBUF`, `IFD`, `BOTH` | `IDELMUXE3.P0`, `IFFDELMUXE3.P0`, both; with nothing on `DDLY` |
/// | `CLKDIVP` on `CLKDIV`'s clock | a bit the database does not name, `CLKDIVP_USED` here, and `IS_CLKDIVP_INVERTED` another; tied, nothing, inverted or not |
fn iserdes_features(
    s: &serdes::Iserdes,
    zinv_c: bool,
    clkdivp: bool,
    pin: &dyn Fn(&str) -> Pin,
) -> Result<Vec<&'static str>, String> {
    let mut out = vec![
        "IDDR_OR_ISERDES.IN_USE",
        "ISERDES.IN_USE",
        "IFF.DDR_CLK_EDGE.OPPOSITE_EDGE",
        "IFF.SRTYPE.SYNC",
    ];
    if s.interface != serdes::Interface::Memory {
        out.push("IDDR.IN_USE");
    }
    const INIT: [&str; 4] = [
        "IFF.ZINIT_Q1",
        "IFF.ZINIT_Q2",
        "IFF.ZINIT_Q3",
        "IFF.ZINIT_Q4",
    ];
    const SRVAL: [&str; 4] = [
        "IFF.ZSRVAL_Q1",
        "IFF.ZSRVAL_Q2",
        "IFF.ZSRVAL_Q3",
        "IFF.ZSRVAL_Q4",
    ];
    for k in 0..4 {
        if !s.init[k] {
            out.push(INIT[k]);
        }
        if !s.srval[k] {
            out.push(SRVAL[k]);
        }
    }
    let mode = format!(
        "ISERDES.{}.{}.W{}",
        s.interface.name(),
        if s.sdr { "SDR" } else { "DDR" },
        s.width
    );
    out.push(named(&ISERDES_FEATURES, &mode).ok_or_else(|| format!("`{mode}`"))?);
    for (on, name) in [
        (s.two_enables, "ISERDES.NUM_CE.N2"),
        (s.slave, "ISERDES.MODE.SLAVE"),
        (s.ofb_used, "ISERDES.OFB_USED"),
        (s.dynamic.0, "ISERDES.DYN_CLK_INV_EN"),
        (s.dynamic.1, "ISERDES.DYN_CLKDIV_INV_EN"),
        (s.clkdiv_inverted, "IS_CLKDIV_INVERTED"),
        (zinv_c, "IFF.ZINV_C"),
    ] {
        if on {
            out.push(name);
        }
    }
    if pin("d") == Pin::Routed && !s.d_inverted {
        out.push("ZINV_D");
    }
    if pin("oclk") == Pin::Routed {
        out.push(if s.oclk_inverted {
            "IFF.INV_OCLK"
        } else {
            "IFF.ZINV_OCLK"
        });
        out.push("O:ODDR_TDDR.IN_USE");
    }
    match s.iobdelay {
        "IBUF" => out.push("IDELMUXE3.P0"),
        "IFD" => out.push("IFFDELMUXE3.P0"),
        "BOTH" => out.extend(["IDELMUXE3.P0", "IFFDELMUXE3.P0"]),
        _ => {}
    }
    if clkdivp {
        out.push("CLKDIVP_USED");
        if s.clkdivp_inverted {
            out.push("IS_CLKDIVP_INVERTED");
        }
    }
    Ok(out)
}

/// The features an `OSERDESE2` needs, and `PDRC-37`. **Every line is
/// Vivado's** (`oserdes_*`):
///
/// | Setting | Features |
/// |---|---|
/// | always | `OSERDES.IN_USE`, `ODDR_TDDR.IN_USE`, `OQUSED`, `ODDR.SRUSED`, `OSERDES.SRTYPE.SYNC`, `OSERDES.TSRTYPE.SYNC`, and `TDDR.SRUSED` unless `DATA_RATE_TQ` is `BUF` — a slave's too, with nothing on its `OQ` |
/// | `IS_CLK_INVERTED` 0 | `ZINV_CLK` and `ODDR.DDR_CLK_EDGE.SAME_EDGE`; an inversion clears both |
/// | rates and width | `OSERDES.DATA_RATE_OQ.<rate>`, `DATA_RATE_TQ.<rate>`, `DATA_WIDTH.<rate>.W<width>`; `W10` and `W14` are bits the database does not name ([`measured_features`]) |
/// | `TRISTATE_WIDTH(4)` | `OSERDES.TRISTATE_WIDTH.W4` |
/// | `INIT_*`, `SRVAL_*` 0 | `ZINIT_OQ`, `ZINIT_TQ`, `ZSRVAL_OQ`, `ZSRVAL_TQ` |
/// | `D<k>` reading 0 | `IS_D<k>_INVERTED`: routed and inverted, tied to the parameter's own value, or unconnected (a zero) |
/// | `T<k>` reading 1 | `ZINV_T<k>`: routed and not inverted, or tied or unconnected reading one; so an unconnected `T` is the inverter on, a zero |
/// | `SLAVE`, `TBYTE_CTL`, `IS_CLKDIV_INVERTED` | `OSERDES.SERDES_MODE.SLAVE`, `TBYTE_CTL` (not named in the database), `IS_CLKDIV_INVERTED` |
fn oserdes_features(
    s: &serdes::Oserdes,
    pin: &dyn Fn(&str) -> Pin,
) -> Result<Vec<&'static str>, String> {
    // The value a data or tristate pin reads, when it is not routed.
    let fixed = |role: &str, inverted: bool| match pin(role) {
        Pin::Routed => None,
        Pin::Fixed(value) => Some(value != inverted),
        Pin::Absent => Some(inverted),
    };
    if fixed("t1", s.t_inverted[0]) == Some(true) && pin("tq") == Pin::Absent {
        return Err(
            "`T1` reads one with nothing on `TQ`, which holds the pad released; Vivado refuses \
             it (PDRC-37)"
                .to_owned(),
        );
    }
    let mut out = vec![
        "OSERDES.IN_USE",
        "ODDR_TDDR.IN_USE",
        "OQUSED",
        "ODDR.SRUSED",
        "OSERDES.SRTYPE.SYNC",
        "OSERDES.TSRTYPE.SYNC",
    ];
    if s.tq_rate != "BUF" {
        out.push("TDDR.SRUSED");
    }
    if !s.clk_inverted {
        out.extend(["ZINV_CLK", "ODDR.DDR_CLK_EDGE.SAME_EDGE"]);
    }
    let rate = if s.oq_sdr { "SDR" } else { "DDR" };
    for name in [
        format!("OSERDES.DATA_RATE_OQ.{rate}"),
        format!("OSERDES.DATA_RATE_TQ.{}", s.tq_rate),
        format!("OSERDES.DATA_WIDTH.{rate}.W{}", s.width),
    ] {
        let found = named(&OLOGIC_FEATURES, &name).or_else(|| named(&OSERDES_FEATURES, &name));
        out.push(found.ok_or_else(|| format!("`{name}`"))?);
    }
    for (on, name) in [
        (s.tristate_four, "OSERDES.TRISTATE_WIDTH.W4"),
        (!s.init.0, "ZINIT_OQ"),
        (!s.init.1, "ZINIT_TQ"),
        (!s.srval.0, "ZSRVAL_OQ"),
        (!s.srval.1, "ZSRVAL_TQ"),
        (s.clkdiv_inverted, "IS_CLKDIV_INVERTED"),
        (s.slave, "OSERDES.SERDES_MODE.SLAVE"),
        (s.tbyte_ctl, "OSERDES.TBYTE_CTL"),
    ] {
        if on {
            out.push(name);
        }
    }
    const D: [&str; 8] = [
        "IS_D1_INVERTED",
        "IS_D2_INVERTED",
        "IS_D3_INVERTED",
        "IS_D4_INVERTED",
        "IS_D5_INVERTED",
        "IS_D6_INVERTED",
        "IS_D7_INVERTED",
        "IS_D8_INVERTED",
    ];
    for (k, name) in D.iter().enumerate() {
        let on = match fixed(D_ROLES[k], s.d_inverted[k]) {
            None => s.d_inverted[k],
            Some(value) => !value,
        };
        if on {
            out.push(name);
        }
    }
    const T: [&str; 4] = ["ZINV_T1", "ZINV_T2", "ZINV_T3", "ZINV_T4"];
    for (k, name) in T.iter().enumerate() {
        let on = match fixed(T_ROLES[k], s.t_inverted[k]) {
            None => !s.t_inverted[k],
            Some(value) => value,
        };
        if on {
            out.push(name);
        }
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
            let (input, serialiser) = match instance.primitive.as_str() {
                "IDDR" | "IDDR_2CLK" => (true, false),
                "ODDR" => (false, false),
                "ISERDESE2" => (true, true),
                "OSERDESE2" => (false, true),
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
            // Which of the two ports on the one `SR` pin is in use: routed,
            // or tied high, which is left unrouted and reads a one.
            let active = |port: &str| {
                pins.iter().any(|p| {
                    p.role == "sr"
                        && p.port == port
                        && (p.signal.is_some() || p.constant == Some(Bit::One))
                })
            };
            let reset = match (active("R"), active("S")) {
                (true, false) => Some(true),
                (false, true) => Some(false),
                (false, false) => None,
                (true, true) => {
                    return Err(IoLogicError::Unsupported {
                        instance: instance.name.clone(),
                        message: format!(
                            "both a set and a reset in use, which Vivado refuses too ({})",
                            if input { "REQP-77" } else { "REQP-129" }
                        ),
                    });
                }
            };
            let none = crate::ir::Attrs::new();
            let cell = m.cells.get(instance.cell);
            let names = match (input, serialiser) {
                (true, false) => iddr_features(&text, &int, reset),
                (false, false) => oddr_features(&text, &int, &state, reset),
                (true, true) => serdes::iserdes(params.unwrap_or(&none)).and_then(|s| {
                    // Where `CLKB` came from, which `flow` left on the cell.
                    let clkb = cell
                        .and_then(|c| c.attrs.get(serdes::CLKB_ATTR))
                        .and_then(AttrValue::as_str)
                        .and_then(serdes::Clkb::from_name)
                        .unwrap_or(serdes::Clkb::Other);
                    let zinv_c = s.clock_inverter(clkb)?;
                    let clkdivp = cell
                        .and_then(|c| c.attrs.get(serdes::CLKDIVP_ATTR))
                        .is_some();
                    iserdes_features(&s, zinv_c, clkdivp, &state)
                }),
                (false, true) => serdes::oserdes(params.unwrap_or(&none))
                    .and_then(|s| oserdes_features(&s, &state)),
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
        let mut iddr = iddr_features(&text, &int, None).unwrap();
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
        // A reset, once refused, and an unknown edge, still refused.
        let reset = iddr_features(&text, &int, Some(true)).unwrap();
        assert!(reset.contains(&"IFF.ZSRVAL_Q1") && reset.contains(&"IFF.ZSRVAL_Q2"));
        let odd = |name: &str, default: &str| match name {
            "DDR_CLK_EDGE" => "BOTH".to_owned(),
            _ => default.to_owned(),
        };
        assert!(iddr_features(&odd, &int, None).is_err());
    }

    fn attrs(list: &[(&str, AttrValue)]) -> crate::ir::Attrs {
        let mut out = crate::ir::Attrs::new();
        for (k, v) in list {
            out.set(*k, v.clone());
        }
        out
    }

    /// Three registers of `testdata/fpga/vivado/iologic/vivado.txt`, by hand,
    /// so that a database-less build still holds the two functions to some of
    /// Vivado's answer: an `ISERDESE2` at its defaults (`iserdes_sweep` `p0`),
    /// an `OSERDESE2` at its defaults with nothing on `T1` to `T4`
    /// (`oserdes_sweep` `p0`), and a slave of `DATA_WIDTH(10)` (`oserdes_wide`
    /// `p0s`). Would catch `IDDR.IN_USE` in `MEMORY`, an unconnected `T` left
    /// without its inverter, a slave's unconnected `D1` read as a one, and
    /// `TRISTATE_WIDTH` named on a width of one; would not catch what only
    /// another setting shows, which `tests/fpga_xray_ioreg.rs` compares.
    #[test]
    fn three_serialisers_vivado_wrote() {
        let routed = |role: &str| match role {
            "d" | "oclk" | "q" | "d1" | "d2" | "d3" | "d4" | "d5" | "d6" | "d7" | "d8" => {
                Pin::Routed
            }
            _ => Pin::Absent,
        };
        let s = serdes::iserdes(&attrs(&[])).unwrap();
        let zinv_c = s.clock_inverter(serdes::Clkb::Inverted).unwrap();
        let mut got = iserdes_features(&s, zinv_c, false, &routed).unwrap();
        got.sort_unstable();
        assert_eq!(
            got,
            [
                "IDDR_OR_ISERDES.IN_USE",
                "IFF.DDR_CLK_EDGE.OPPOSITE_EDGE",
                "IFF.SRTYPE.SYNC",
                "IFF.ZINIT_Q1",
                "IFF.ZINIT_Q2",
                "IFF.ZINIT_Q3",
                "IFF.ZINIT_Q4",
                "IFF.ZINV_C",
                "IFF.ZINV_OCLK",
                "IFF.ZSRVAL_Q1",
                "IFF.ZSRVAL_Q2",
                "IFF.ZSRVAL_Q3",
                "IFF.ZSRVAL_Q4",
                "ISERDES.IN_USE",
                "ISERDES.MEMORY.DDR.W4",
                "ISERDES.NUM_CE.N2",
                "O:ODDR_TDDR.IN_USE",
                "ZINV_D",
            ]
        );
        let s = serdes::oserdes(&attrs(&[])).unwrap();
        let mut got = oserdes_features(&s, &routed).unwrap();
        got.sort_unstable();
        assert_eq!(
            got,
            [
                "ODDR.DDR_CLK_EDGE.SAME_EDGE",
                "ODDR.SRUSED",
                "ODDR_TDDR.IN_USE",
                "OQUSED",
                "OSERDES.DATA_RATE_OQ.DDR",
                "OSERDES.DATA_RATE_TQ.DDR",
                "OSERDES.DATA_WIDTH.DDR.W4",
                "OSERDES.IN_USE",
                "OSERDES.SRTYPE.SYNC",
                "OSERDES.TRISTATE_WIDTH.W4",
                "OSERDES.TSRTYPE.SYNC",
                "TDDR.SRUSED",
                "ZINIT_OQ",
                "ZINIT_TQ",
                "ZINV_CLK",
                "ZSRVAL_OQ",
                "ZSRVAL_TQ",
            ]
        );
        let slave = serdes::oserdes(&attrs(&[
            ("DATA_WIDTH", AttrValue::Int(10)),
            ("TRISTATE_WIDTH", AttrValue::Int(1)),
            ("SERDES_MODE", AttrValue::String("SLAVE".into())),
        ]))
        .unwrap();
        let wired = |role: &str| match role {
            "d3" | "d4" => Pin::Routed,
            _ => Pin::Absent,
        };
        let mut got = oserdes_features(&slave, &wired).unwrap();
        got.sort_unstable();
        assert_eq!(
            got,
            [
                "IS_D1_INVERTED",
                "IS_D2_INVERTED",
                "IS_D5_INVERTED",
                "IS_D6_INVERTED",
                "IS_D7_INVERTED",
                "IS_D8_INVERTED",
                "ODDR.DDR_CLK_EDGE.SAME_EDGE",
                "ODDR.SRUSED",
                "ODDR_TDDR.IN_USE",
                "OQUSED",
                "OSERDES.DATA_RATE_OQ.DDR",
                "OSERDES.DATA_RATE_TQ.DDR",
                "OSERDES.DATA_WIDTH.DDR.W10",
                "OSERDES.IN_USE",
                "OSERDES.SERDES_MODE.SLAVE",
                "OSERDES.SRTYPE.SYNC",
                "OSERDES.TSRTYPE.SYNC",
                "TDDR.SRUSED",
                "ZINIT_OQ",
                "ZINIT_TQ",
                "ZINV_CLK",
                "ZSRVAL_OQ",
                "ZSRVAL_TQ",
            ]
        );
        // `PDRC-37`: an inverted `T1` with nothing on `TQ`.
        let inverted = serdes::oserdes(&attrs(&[("IS_T1_INVERTED", AttrValue::Int(1))])).unwrap();
        let refused = oserdes_features(&inverted, &routed).unwrap_err();
        assert!(refused.contains("PDRC-37"), "{refused}");
    }

    /// The bits [`measured_features`] adds are where Vivado wrote them on both
    /// halves: `26_119` and its mirror `27_08`, seen on an `_N` ball. Would
    /// catch the mirror mis-stated; the rest is `tests/fpga_xray_ioreg.rs`'s.
    #[test]
    fn the_measured_bits_mirror_onto_half_one() {
        let features = measured_features("RIOI3");
        let find = |name: &str| features.iter().find(|f| f.name == name).unwrap();
        assert_eq!(
            find("ILOGIC_Y1.IS_CLKDIV_INVERTED").ones,
            [ConfigBit::new(27, 8)]
        );
        assert_eq!(
            find("OLOGIC_Y1.OSERDES.TBYTE_CTL").ones,
            [ConfigBit::new(33, 47)]
        );
        assert!(measured_features("CLBLL_L").is_empty());
    }
}
