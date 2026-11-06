//! What Vivado allows of an `ISERDESE2` and an `OSERDESE2`, and what each
//! setting is, read once for the flow (`fpga::flow`, before placing) and the
//! bitstream (`xray::iologic`, after routing).
//!
//! # Where every rule here comes from
//!
//! **Vivado 2026.1, one setting a pad**, `testdata/fpga/vivado/iologic/`
//! (`tools/vivado-parity/gen_iologic.py`): every rule below is a design Vivado
//! refused, by the name given, and every setting accepted is one it built and
//! `tests/fpga_xray_ioreg.rs` compares. A setting no design asked is refused
//! here as not measured, rather than guessed.
//!
//! | Rule | What |
//! |---|---|
//! | `AVAL-69` | an `OSERDESE2` wider than four with `TRISTATE_WIDTH` 4 |
//! | `AVAL-74` | `DATA_RATE_TQ` of `SDR` or `BUF` with `TRISTATE_WIDTH` 4 |
//! | `PDRC-26` | `CLK` and `CLKB` one clock, with inversions that leave them the same edge (below) |
//! | `PDRC-37` | `T1` high and `TQ` unconnected: the pad held released |
//! | `INBB-3` | no `CLKB` at a double data rate, which synthesis leaves a black box |
//! | `THRU-2` | `OFB`, or `TFB` straight into a flip-flop on a pad, read by the fabric (`xray::iologic`) |
//! | `Route 35-7` | `DDLY` from anything but an `IDELAYE2`'s `DATAOUT` (`fpga::flow`) |
//! | `PDIL-1` | a delay into `DDLY` that the fabric reads too, with `IOBDELAY` other than `BOTH` (`xray::idelay`) |
//!
//! A delay on `DDLY` is otherwise built in every `IOBDELAY`, `NONE` included,
//! and writes the delay's features and the serialiser's as each writes them
//! alone (`testdata/fpga/vivado/idelay/`, `ddly_*`). A delay of the fabric on
//! `DDLY` Vivado builds, and so does this flow since 2026-11-07, the two in
//! one tile and every feature Vivado's (`ddly_datain`); it was refused before,
//! when nothing placed the two together.
//!
//! **`PDRC-26` is a fit, and here is what it fits.** Of the 70 combinations
//! of `CLKB` (`~CLK`, `CLK`, unconnected, a second clock), `IS_CLK_INVERTED`,
//! `IS_CLKB_INVERTED` and five interface types that were asked, call `b` the
//! inversion `CLKB` carries against `CLK`'s net (`~CLK` is one, and
//! `IS_CLKB_INVERTED` another). On one net, Vivado refuses at a double data
//! rate when neither `IS_CLK_INVERTED` nor `b` is set, and at a single rate
//! when the two are equal. A second clock is never refused.
//!
//! **And what a clock inversion writes is not what its name says.** At a
//! double data rate `IS_CLK_INVERTED` and `IS_CLKB_INVERTED` set no bit at all;
//! at a single rate `IFF.ZINV_C` is cleared only when `IS_CLK_INVERTED` is set
//! and `CLKB` is unconnected or carries `b`. `IS_OCLKB_INVERTED` and
//! `IS_CLKDIVP_INVERTED` set nothing either, nor does what `OCLKB` is wired to.
//! Measured, in `iserdes_clocks` and the exploration before it; why is not
//! known, and nothing here has been on a part.

use crate::ir::{AttrValue, Attrs};

/// The attribute `fpga::flow` leaves on an `ISERDESE2`'s cell to say where
/// its `CLKB` came from ([`Clkb::name`]), since what reaches the pin is then
/// `CLK`'s net either way.
pub const CLKB_ATTR: &str = "clkb_from";

/// And the one it leaves when `CLKDIVP` is on `CLKDIV`'s clock, which sets a
/// bit of its own (`iserdes_clkdivp`) though the pin is not routed here: it is
/// `CLKDIV`'s wire.
pub const CLKDIVP_ATTR: &str = "clkdivp_used";

/// An `ISERDESE2`'s interface type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interface {
    /// `MEMORY`, the default.
    Memory,
    /// `MEMORY_DDR3`.
    MemoryDdr3,
    /// `MEMORY_QDR`.
    MemoryQdr,
    /// `NETWORKING`.
    Networking,
    /// `OVERSAMPLE`.
    Oversample,
}

impl Interface {
    /// The database's spelling, as in `ISERDES.MEMORY_QDR.DDR.W4`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Interface::Memory => "MEMORY",
            Interface::MemoryDdr3 => "MEMORY_DDR3",
            Interface::MemoryQdr => "MEMORY_QDR",
            Interface::Networking => "NETWORKING",
            Interface::Oversample => "OVERSAMPLE",
        }
    }
}

/// Where an `ISERDESE2`'s `CLKB` comes from, against its `CLK`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clkb {
    /// `~CLK`: the same net, inverted.
    Inverted,
    /// `CLK` itself.
    Same,
    /// Nothing.
    Open,
    /// Another net.
    Other,
}

impl Clkb {
    /// The spelling the flow leaves on the cell (`clkb_from`), for the
    /// bitstream to read back.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Clkb::Inverted => "inverted",
            Clkb::Same => "same",
            Clkb::Open => "open",
            Clkb::Other => "other",
        }
    }

    /// The reverse of [`Clkb::name`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Clkb> {
        [Clkb::Inverted, Clkb::Same, Clkb::Open, Clkb::Other]
            .into_iter()
            .find(|c| c.name() == name)
    }
}

/// An `ISERDESE2`'s parameters, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Iserdes {
    /// `DATA_RATE` is `SDR`.
    pub sdr: bool,
    /// `DATA_WIDTH`.
    pub width: i64,
    /// `INTERFACE_TYPE`.
    pub interface: Interface,
    /// `NUM_CE` is 2.
    pub two_enables: bool,
    /// `SERDES_MODE` is `SLAVE`.
    pub slave: bool,
    /// `OFB_USED` is `TRUE`.
    pub ofb_used: bool,
    /// `IOBDELAY`: `NONE`, `IBUF`, `IFD` or `BOTH`.
    pub iobdelay: &'static str,
    /// `DYN_CLK_INV_EN` and `DYN_CLKDIV_INV_EN`.
    pub dynamic: (bool, bool),
    /// `INIT_Q1` to `INIT_Q4`.
    pub init: [bool; 4],
    /// `SRVAL_Q1` to `SRVAL_Q4`.
    pub srval: [bool; 4],
    /// `IS_CLK_INVERTED`, `IS_CLKB_INVERTED`.
    pub clk_inverted: (bool, bool),
    /// `IS_CLKDIV_INVERTED`.
    pub clkdiv_inverted: bool,
    /// `IS_CLKDIVP_INVERTED`.
    pub clkdivp_inverted: bool,
    /// `IS_D_INVERTED`.
    pub d_inverted: bool,
    /// `IS_OCLK_INVERTED`.
    pub oclk_inverted: bool,
}

/// An `OSERDESE2`'s parameters, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Oserdes {
    /// `DATA_RATE_OQ` is `SDR`.
    pub oq_sdr: bool,
    /// `DATA_RATE_TQ`: `DDR`, `SDR` or `BUF`.
    pub tq_rate: &'static str,
    /// `DATA_WIDTH`.
    pub width: i64,
    /// `TRISTATE_WIDTH` is 4.
    pub tristate_four: bool,
    /// `SERDES_MODE` is `SLAVE`.
    pub slave: bool,
    /// `TBYTE_CTL` is `TRUE`.
    pub tbyte_ctl: bool,
    /// `INIT_OQ`, `INIT_TQ`, `SRVAL_OQ`, `SRVAL_TQ`.
    pub init: (bool, bool),
    /// `SRVAL_OQ`, `SRVAL_TQ`.
    pub srval: (bool, bool),
    /// `IS_CLK_INVERTED`.
    pub clk_inverted: bool,
    /// `IS_CLKDIV_INVERTED`.
    pub clkdiv_inverted: bool,
    /// `IS_D1_INVERTED` to `IS_D8_INVERTED`.
    pub d_inverted: [bool; 8],
    /// `IS_T1_INVERTED` to `IS_T4_INVERTED`.
    pub t_inverted: [bool; 4],
}

fn text(params: &Attrs, name: &str, default: &str) -> String {
    params
        .get(name)
        .and_then(AttrValue::as_str)
        .unwrap_or(default)
        .to_ascii_uppercase()
}

fn int(params: &Attrs, name: &str, default: i64) -> i64 {
    params
        .get(name)
        .and_then(AttrValue::as_int)
        .unwrap_or(default)
}

fn flag(params: &Attrs, name: &str) -> bool {
    int(params, name, 0) & 1 == 1
}

fn boolean(params: &Attrs, name: &str) -> Result<bool, String> {
    match text(params, name, "FALSE").as_str() {
        "TRUE" => Ok(true),
        "FALSE" => Ok(false),
        other => Err(format!(
            "`{name}(\"{other}\")`, which is neither TRUE nor FALSE"
        )),
    }
}

/// Reads an `ISERDESE2`'s parameters, refusing a value Vivado was not asked
/// about with what it is.
///
/// # Errors
///
/// The setting, in words, when it is not one `iserdes_*` measured.
pub fn iserdes(params: &Attrs) -> Result<Iserdes, String> {
    let sdr = match text(params, "DATA_RATE", "DDR").as_str() {
        "SDR" => true,
        "DDR" => false,
        other => return Err(format!("`DATA_RATE(\"{other}\")`")),
    };
    let interface = match text(params, "INTERFACE_TYPE", "MEMORY").as_str() {
        "MEMORY" => Interface::Memory,
        "MEMORY_DDR3" => Interface::MemoryDdr3,
        "MEMORY_QDR" => Interface::MemoryQdr,
        "NETWORKING" => Interface::Networking,
        "OVERSAMPLE" => Interface::Oversample,
        other => return Err(format!("`INTERFACE_TYPE(\"{other}\")`")),
    };
    let width = int(params, "DATA_WIDTH", 4);
    // The widths Vivado built: every one the database names, which is all a
    // `NETWORKING` block has, and four in every other interface.
    let measured = match (interface, sdr) {
        (Interface::Networking, true) => (2..=8).contains(&width),
        (Interface::Networking, false) => matches!(width, 4 | 6 | 8 | 10 | 14),
        (_, false) => width == 4,
        (_, true) => false,
    };
    if !measured {
        return Err(format!(
            "`INTERFACE_TYPE(\"{}\")` at `DATA_RATE(\"{}\")` and `DATA_WIDTH({width})`, which \
             was not measured",
            interface.name(),
            if sdr { "SDR" } else { "DDR" }
        ));
    }
    let two_enables = match int(params, "NUM_CE", 2) {
        1 => false,
        2 => true,
        other => return Err(format!("`NUM_CE({other})`")),
    };
    let slave = match text(params, "SERDES_MODE", "MASTER").as_str() {
        "MASTER" => false,
        "SLAVE" => true,
        other => return Err(format!("`SERDES_MODE(\"{other}\")`")),
    };
    let iobdelay = match text(params, "IOBDELAY", "NONE").as_str() {
        "NONE" => "NONE",
        "IBUF" => "IBUF",
        "IFD" => "IFD",
        "BOTH" => "BOTH",
        other => return Err(format!("`IOBDELAY(\"{other}\")`")),
    };
    let quad = |prefix: &str| [1, 2, 3, 4].map(|k| flag(params, &format!("{prefix}{k}")));
    Ok(Iserdes {
        sdr,
        width,
        interface,
        two_enables,
        slave,
        ofb_used: boolean(params, "OFB_USED")?,
        iobdelay,
        dynamic: (
            boolean(params, "DYN_CLK_INV_EN")?,
            boolean(params, "DYN_CLKDIV_INV_EN")?,
        ),
        init: quad("INIT_Q"),
        srval: quad("SRVAL_Q"),
        clk_inverted: (
            flag(params, "IS_CLK_INVERTED"),
            flag(params, "IS_CLKB_INVERTED"),
        ),
        clkdiv_inverted: flag(params, "IS_CLKDIV_INVERTED"),
        clkdivp_inverted: flag(params, "IS_CLKDIVP_INVERTED"),
        d_inverted: flag(params, "IS_D_INVERTED"),
        oclk_inverted: flag(params, "IS_OCLK_INVERTED"),
    })
}

impl Iserdes {
    /// Whether `CLKB` from `clkb` is one Vivado builds (`PDRC-26`, `INBB-3`),
    /// and then whether `IFF.ZINV_C` is set. See the module documentation.
    ///
    /// # Errors
    ///
    /// The refusal, with Vivado's rule.
    pub fn clock_inverter(&self, clkb: Clkb) -> Result<bool, String> {
        let (clk, clkb_flag) = self.clk_inverted;
        // The inversion `CLKB` carries against `CLK`'s net.
        let b = clkb_flag != (clkb == Clkb::Inverted);
        if clkb == Clkb::Open && !self.sdr {
            return Err(
                "no `CLKB` at a double data rate, which Vivado's synthesis leaves a black box \
                 (INBB-3)"
                    .to_owned(),
            );
        }
        if matches!(clkb, Clkb::Inverted | Clkb::Same) {
            let refused = if self.sdr { clk == b } else { !clk && !b };
            if refused {
                return Err(
                    "`CLK` and `CLKB` are one clock with inversions that leave them the same \
                     edge, which Vivado refuses (PDRC-26)"
                        .to_owned(),
                );
            }
        }
        Ok(!(self.sdr && clk && (clkb == Clkb::Open || b)))
    }
}

/// Reads an `OSERDESE2`'s parameters, with Vivado's refusals of them.
///
/// # Errors
///
/// `AVAL-69`, `AVAL-74`, or a setting that was not measured.
pub fn oserdes(params: &Attrs) -> Result<Oserdes, String> {
    let oq_sdr = match text(params, "DATA_RATE_OQ", "DDR").as_str() {
        "SDR" => true,
        "DDR" => false,
        other => return Err(format!("`DATA_RATE_OQ(\"{other}\")`")),
    };
    let tq_rate = match text(params, "DATA_RATE_TQ", "DDR").as_str() {
        "DDR" => "DDR",
        "SDR" => "SDR",
        "BUF" => "BUF",
        other => return Err(format!("`DATA_RATE_TQ(\"{other}\")`")),
    };
    let width = int(params, "DATA_WIDTH", 4);
    let measured = if oq_sdr {
        (2..=8).contains(&width)
    } else {
        matches!(width, 4 | 6 | 8 | 10 | 14)
    };
    if !measured {
        return Err(format!(
            "`DATA_RATE_OQ(\"{}\")` at `DATA_WIDTH({width})`, which was not measured",
            if oq_sdr { "SDR" } else { "DDR" }
        ));
    }
    let tristate_four = match int(params, "TRISTATE_WIDTH", 4) {
        1 => false,
        4 => true,
        other => return Err(format!("`TRISTATE_WIDTH({other})`")),
    };
    if tristate_four && width > 4 {
        return Err(format!(
            "`DATA_WIDTH({width})` needs `TRISTATE_WIDTH(1)`, as Vivado says (AVAL-69)"
        ));
    }
    if tristate_four && tq_rate != "DDR" {
        return Err(format!(
            "`DATA_RATE_TQ(\"{tq_rate}\")` needs `TRISTATE_WIDTH(1)`, as Vivado says (AVAL-74)"
        ));
    }
    let slave = match text(params, "SERDES_MODE", "MASTER").as_str() {
        "MASTER" => false,
        "SLAVE" => true,
        other => return Err(format!("`SERDES_MODE(\"{other}\")`")),
    };
    if boolean(params, "TBYTE_SRC")? {
        return Err("`TBYTE_SRC(\"TRUE\")`, which was not measured".to_owned());
    }
    Ok(Oserdes {
        oq_sdr,
        tq_rate,
        width,
        tristate_four,
        slave,
        tbyte_ctl: boolean(params, "TBYTE_CTL")?,
        init: (flag(params, "INIT_OQ"), flag(params, "INIT_TQ")),
        srval: (flag(params, "SRVAL_OQ"), flag(params, "SRVAL_TQ")),
        clk_inverted: flag(params, "IS_CLK_INVERTED"),
        clkdiv_inverted: flag(params, "IS_CLKDIV_INVERTED"),
        d_inverted: [1, 2, 3, 4, 5, 6, 7, 8].map(|k| flag(params, &format!("IS_D{k}_INVERTED"))),
        t_inverted: [1, 2, 3, 4].map(|k| flag(params, &format!("IS_T{k}_INVERTED"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(list: &[(&str, AttrValue)]) -> Attrs {
        let mut out = Attrs::new();
        for (k, v) in list {
            out.set(*k, v.clone());
        }
        out
    }

    /// Every combination of `CLKB` and its inversions Vivado was asked, in
    /// two of the five interfaces, with what it said: built and `ZINV_C`, or
    /// refused. Would catch the fit mis-stated for either rate; would not catch
    /// a rule that is wrong in the same way as this table, which is Vivado's
    /// (`iserdes_clocks` and the designs refused beside it hold the build).
    #[test]
    fn the_clock_rules_are_the_combinations_vivado_answered() {
        use Clkb::{Inverted, Open, Other, Same};
        // (CLKB, IS_CLK_INVERTED, IS_CLKB_INVERTED, SDR answer, DDR answer):
        // `Some(zinv_c)` built, `None` refused.
        let table = [
            (Inverted, 0, 0, Some(true), Some(true)),
            (Inverted, 0, 1, None, None),
            (Inverted, 1, 0, None, Some(true)),
            (Inverted, 1, 1, Some(true), Some(true)),
            (Same, 0, 0, None, None),
            (Same, 0, 1, Some(true), Some(true)),
            (Same, 1, 0, Some(true), Some(true)),
            (Same, 1, 1, None, Some(true)),
            (Open, 0, 0, Some(true), None),
            (Open, 1, 0, Some(false), None),
            (Other, 0, 0, Some(true), Some(true)),
            (Other, 0, 1, Some(true), Some(true)),
            (Other, 1, 0, Some(true), Some(true)),
            (Other, 1, 1, Some(false), Some(true)),
        ];
        for (clkb, ic, icb, sdr, ddr) in table {
            for (rate, want) in [("SDR", sdr), ("DDR", ddr)] {
                let s = iserdes(&params(&[
                    ("INTERFACE_TYPE", AttrValue::String("NETWORKING".into())),
                    ("DATA_RATE", AttrValue::String(rate.into())),
                    ("IS_CLK_INVERTED", AttrValue::Int(ic)),
                    ("IS_CLKB_INVERTED", AttrValue::Int(icb)),
                ]))
                .unwrap();
                let got = s.clock_inverter(clkb).ok();
                assert_eq!(got, want, "{clkb:?} {ic}{icb} {rate}");
            }
        }
    }

    /// `AVAL-69` and `AVAL-74`, and the settings beside them that are built.
    #[test]
    fn the_tristate_width_rules_are_vivados() {
        let with = |list: &[(&str, AttrValue)]| oserdes(&params(list));
        let s = |v: &str| AttrValue::String(v.into());
        assert!(with(&[]).is_ok());
        let wide = with(&[("DATA_WIDTH", AttrValue::Int(8))]).unwrap_err();
        assert!(wide.contains("AVAL-69"), "{wide}");
        let sdr = with(&[("DATA_RATE_TQ", s("SDR"))]).unwrap_err();
        assert!(sdr.contains("AVAL-74"), "{sdr}");
        assert!(
            with(&[
                ("DATA_WIDTH", AttrValue::Int(8)),
                ("TRISTATE_WIDTH", AttrValue::Int(1)),
                ("DATA_RATE_TQ", s("BUF")),
            ])
            .is_ok()
        );
        assert!(with(&[("DATA_WIDTH", AttrValue::Int(5))]).is_err());
    }
}
