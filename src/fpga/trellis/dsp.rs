//! Why this backend puts no `MULT18X18D` on the fabric, and the refusal
//! that says so.
//!
//! `ecp5.dev` declares the block and [`crate::fpga::primitives`] maps a
//! multiply onto it, which is right for the nextpnr route. Reticle's
//! *own* ECP5 place, route and bitstream cannot build one, for the same
//! reason the 7-series cannot build a `DSP48E1`
//! ([`crate::fpga::xray::dsp`] has that account): **there is no site**.
//! `trellis/sites.rs` names no DSP bel and holds no table of its pins,
//! so a netlist with a multiplier in it reaches the placer with nothing
//! to place it on.
//!
//! What that looked like before this refusal existed is worth writing
//! down, because the symptom pointed somewhere else entirely. The cell
//! mapped, the placer ignored it, and the flow stopped at the driverless
//! check with
//!
//! ```text
//! error: nothing drives 18 signal(s) that something reads ... `p$ff$mul[0]`
//! is read by p$ff$ff0.DI[0] and nothing drives it
//! ```
//!
//! which reads as a routing fault on this family — the message even
//! explains that an unrouted ECP5 input reads as a one — and is nothing
//! of the kind. The multiplier was never built.
//!
//! Only an accident kept anyone from meeting it. Verilog widens both
//! operands of `a * b` to the width of what it is assigned to, so a
//! useful multiply arrives as a cell as wide as its *product* — 29 by 29
//! for a 15-bit by 14-bit one — and [`crate::fpga::primitives`] only
//! looks through that widening for a DSP shape declared `signed`.
//! `xc7.dev` declares one; `ecp5.dev`, `gowin.dev` and `generic.dev` do
//! not. So on this family almost nothing ever fitted, and what does fit
//! — `18 x 18` written into eighteen bits, which throws the top half
//! away — is not what anyone writes on purpose.
//!
//! The command line turns DSP inference off whenever a bitstream is
//! asked for, whatever the family, because no backend here can place
//! one; this refusal is for a `MULT18X18D` that reaches the backend
//! anyway, instantiated by hand in the source.

use crate::ir::{CellKind, Design, ModuleId};

/// The primitive this backend cannot place.
pub const DSP_PRIMITIVE: &str = "MULT18X18D";

/// Why a netlist cannot become a bitstream through this backend, naming
/// every `MULT18X18D` cell of `top` in it, or `None` when it has none.
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
        "{} `{DSP_PRIMITIVE}` cell(s) ({}) cannot be placed by Reticle's own ECP5 \
         backend: `trellis/sites.rs` names no DSP bel and holds no table of its pins, so \
         there is nowhere on the loaded fabric to put one. Build without `--bitstream` and \
         let nextpnr place it from the exported netlist, or write the multiply as `*` so \
         that the `--bitstream` flow keeps it in lookup tables",
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
        let a = b.input("a", Type::bits(18));
        let p = b.output("p", Type::bits(36));
        let a_e = b.net(a);
        let kind = if with_dsp {
            CellKind::Blackbox(Name::new(DSP_PRIMITIVE))
        } else {
            CellKind::Blackbox(Name::new("LUT4"))
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

    /// A hand-instantiated multiplier is refused by name, and says what
    /// to do instead.
    ///
    /// What this would catch: the refusal going missing, which would
    /// put the driverless check in its place — and that one blames the
    /// router on a family where an unrouted input reads as a one, which
    /// is a long way from "this backend has no DSP site".
    #[test]
    fn a_mult18x18d_is_refused_by_name() {
        let (design, top) = build(true);
        let message = dsp_refusal(&design, top).expect("a refusal");
        assert!(message.contains("MULT18X18D"), "{message}");
        assert!(message.contains("nextpnr"), "{message}");
        assert!(
            message.contains("`m`") || message.contains("(m)"),
            "{message}"
        );
    }

    #[test]
    fn a_netlist_without_one_is_not_refused() {
        let (design, top) = build(false);
        assert!(dsp_refusal(&design, top).is_none());
    }
}
