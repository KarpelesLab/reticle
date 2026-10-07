//! Why this backend puts no `DSP48E1` on the fabric, and the refusal
//! that says so.
//!
//! `xc7.dev` declares the block and `fpga::primitives` maps a multiply
//! onto it, which is right for the Vivado route. Reticle's *own* 7-series
//! place, route and bitstream cannot build one, and a netlist holding one
//! is refused here by name rather than reaching the placer, whose generic
//! answer — "the design needs 1 `dsp` site(s) and the part has 0" — is
//! true of the loaded fabric and false of the part, which has ninety.
//!
//! What stands in the way, read off the database rather than assumed
//! (`docs/fpga-xray.md` has the account):
//!
//! - **No site.** `parse::site_kind` would call a `DSP48E1` a `dsp`, but
//!   the loader never gets that far: it names a tile's bels by the prefix
//!   of its features, and every `DSP_L`/`DSP_R` feature is
//!   `DSP48.DSP_0.*` or `DSP48.DSP_1.*`, so both blocks of a tile become
//!   **one** site called `DSP48`, of kind `other`, with **no pins** —
//!   measured, and pinned by `tests/fpga_dsp.rs`. `sites.rs` has no table
//!   of its pins either. The pins themselves are
//!   documented: `ppips_dsp_l.db` is 560 `always` lines joining every site
//!   pin to a tile wire (`DSP_L.DSP_0_A0.DSP_IMUX23_0 always`), the same
//!   shape a slice's pins have.
//! - **A configuration this database cannot finish.** `segbits_dsp_l.db`
//!   has the registers (`AREG_0`, `ZMREG`, `ZPREG` …), the input
//!   inversions and the pattern detector, from the fuzzer
//!   `100-dsp-mskpat`. That fuzzer also varied `USE_MULT` over `NONE`,
//!   `MULTIPLY` and `DYNAMIC` and tagged it, and **no `USE_MULT` feature
//!   survived into the database**; nor did `USE_PATTERN_DETECT`. Whether
//!   a block with none of those bits set multiplies is exactly what a
//!   wrong guess would get silently wrong.
//! - **Zeros the router cannot make.** An unrouted 7-series input reads
//!   one (`ppips_int_l.db`: `INT_L.IMUX_L0.VCC_WIRE default`), so a zero
//!   on a pin has to be routed from `GND_WIRE` through `GFAN0`/`GFAN1`.
//!   `OPMODE`, `ALUMODE`, `INMODE` and `CARRYIN` could take their zeros
//!   from the block's own inverters (`ZIS_*_INVERTED`), but `CARRYINSEL[1:0]`
//!   has no inverter and no local tie, and the router has no constant
//!   source at all.
//! - **No oracle.** Every other tile this backend configures was compared
//!   with what Vivado wrote for the same cell; the Vivado bitstream
//!   `prjxray-db` ships is switches to LEDs and holds no DSP.
//!
//! The command line therefore turns DSP inference off when it is asked for
//! this backend's bitstream, and a multiply goes to lookup tables, which
//! are slower and correct; this refusal is for a `DSP48E1` that reaches
//! the backend anyway, instantiated by hand in the source.

use crate::ir::{CellKind, Design, ModuleId};

/// The primitive this backend cannot place.
pub const DSP_PRIMITIVE: &str = "DSP48E1";

/// Why a netlist cannot become a bitstream through this backend, naming
/// every `DSP48E1` cell of `top` in it, or `None` when it has none.
pub fn dsp_refusal(design: &Design, top: ModuleId) -> Option<String> {
    let module = design.modules.get(top)?;
    let mut cells: Vec<&str> = module
        .cells
        .iter()
        .filter(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == DSP_PRIMITIVE))
        .map(|(_, c)| c.name.as_str())
        .collect();
    if cells.is_empty() {
        return None;
    }
    cells.sort_unstable();
    Some(format!(
        "{} `{DSP_PRIMITIVE}` cell(s) ({}) cannot be placed by Reticle's own 7-series \
         backend: prjxray-db documents the block's pins and registers but no `USE_MULT` \
         bits, and a zero on `CARRYINSEL` needs a constant route this router cannot make \
         (docs/fpga-xray.md has the account). Build without `--bitstream` and let Vivado \
         place it from the exported netlist, or write the multiply as `*` so that the \
         `--bitstream` flow keeps it in lookup tables",
        cells.len(),
        cells.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{Name, Type};
    use crate::source::{SourceMap, Span};

    fn build(with_dsp: bool) -> (Design, ModuleId) {
        let mut map = SourceMap::new();
        let file = map.add("t", "").unwrap();
        let mut b = ModuleBuilder::new("top", Span::new(file, 0, 0));
        let a = b.input("a", Type::bits(25));
        let p = b.output("p", Type::bits(48));
        let a_e = b.net(a);
        let kind = if with_dsp {
            CellKind::Blackbox(Name::new(DSP_PRIMITIVE))
        } else {
            CellKind::Blackbox(Name::new("LUT6"))
        };
        b.cell(
            "m",
            kind,
            vec![(Name::new("A"), a_e)],
            vec![(Name::new("P"), p)],
        );
        let mut design = Design::new();
        let top = design.add_module(b.finish());
        (design, top)
    }

    #[test]
    fn a_dsp48e1_is_refused_by_name() {
        let (design, top) = build(true);
        let message = dsp_refusal(&design, top).expect("refused");
        assert!(message.contains("1 `DSP48E1` cell(s) (m)"), "{message}");
        assert!(message.contains("without `--bitstream`"), "{message}");
        let (design, top) = build(false);
        assert_eq!(dsp_refusal(&design, top), None);
    }
}
