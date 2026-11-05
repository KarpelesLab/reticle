//! The reset of the digitally controlled impedance, `DCIRESET`, which the
//! XC7A100T has in its configuration tile although none of its banks has
//! DCI.
//!
//! # What Vivado writes for it
//!
//! **Vivado 2026.1**, `testdata/fpga/vivado/idelay/dcireset_*`, decoded:
//! `CFG_CENTER_MID.DCIRESET.ENABLED` and the routes of its two pins, and
//! nothing else. `RST` comes from the interconnect's `IMUX35` by way of the
//! configuration tile's `CFG_CENTER_IMUX35_8` and a `VFRAME`; `LOCKED` leaves
//! by `CFG_CENTER_LOGIC_OUTS_B21_9`. A reset tied low is grounded
//! (`IMUX_L35.GFAN0`), one left open is left, and a `DCIRESET` whose `LOCKED`
//! nothing reads is taken away, as Vivado takes it away.
//!
//! The database has no `ppips` file for the configuration tile, so the two
//! joins between the pins and the tile's interconnect wires are declared here,
//! from `tile_type_CFG_CENTER_MID.json`'s pips, as `super::xadc` declares the
//! XADC's. **What a `DCIRESET` does on this part, with no DCI to reset, has
//! not been asked of anything.**

use super::sites::{BelPin, ExtraBel, PassThrough};

/// The primitive.
pub(super) const PRIMITIVE: &str = "DCIRESET";
/// Its site kind, in [`BelRole`](crate::fpga::BelRole)'s vocabulary.
pub(super) const KIND: &str = "dcireset";

/// The tile that holds it.
const TILE: &str = "CFG_CENTER_MID";

/// `(role, pin wire, interconnect wire, input)`, from
/// `tile_type_CFG_CENTER_MID.json`.
const PINS: [(&str, &str, &str, bool); 2] = [
    (
        "rst",
        "CFG_CENTER_DCIRESET_RST",
        "CFG_CENTER_IMUX35_8",
        true,
    ),
    (
        "locked",
        "CFG_CENTER_DCIRESET_LOCKED",
        "CFG_CENTER_LOGIC_OUTS_B21_9",
        false,
    ),
];

/// The bel of the configuration tile, or nothing.
pub(super) fn bels(tile_type: &str) -> Vec<ExtraBel> {
    if tile_type != TILE {
        return Vec::new();
    }
    vec![ExtraBel {
        name: "DCIRESET".to_owned(),
        kind: KIND,
        pins: PINS
            .iter()
            .map(|(role, wire, _, _)| BelPin {
                role,
                wire: (*wire).to_owned(),
            })
            .collect(),
        cells: vec![(PRIMITIVE, vec!["DCIRESET.ENABLED".to_owned()])],
    }]
}

/// The joins between its pins and the tile's interconnect wires, which cost
/// nothing. Declared only for a design with one (`XrayOptions::dci_reset`).
pub(super) fn pass_throughs(tile_type: &str) -> Vec<PassThrough> {
    if tile_type != TILE {
        return Vec::new();
    }
    PINS.iter()
        .map(|(_, pin, joined, input)| {
            let (to, from) = if *input { (pin, joined) } else { (joined, pin) };
            PassThrough {
                to: (*to).to_owned(),
                from: (*from).to_owned(),
                features: Vec::new(),
            }
        })
        .collect()
}

/// A reset tied low is grounded, as Vivado grounds it; one left open is left.
pub(super) fn wants(
    instance: &crate::fpga::place::Instance,
    site: &crate::fpga::arch::ArchSite,
    netlist: &crate::fpga::place::Netlist,
    wants: &mut Vec<super::bram::Want>,
) {
    let low = instance.pins.iter().map(|p| &netlist.pins[*p]).any(|p| {
        p.role == "rst" && p.signal.is_none() && p.constant == Some(crate::logic::Bit::Zero)
    });
    if low && let Some(node) = site.pin("rst") {
        wants.push((
            super::bram::TiePolicy::Zero,
            instance.name.clone(),
            "rst".to_owned(),
            node,
        ));
    }
}

/// Takes away every `DCIRESET` whose `LOCKED` nothing reads, as Vivado does,
/// and says how many: read as `xray::sweep_unread_config` reads its cells'.
///
/// # Errors
///
/// The module's, if its bits cannot be viewed.
pub fn unread_dci_resets(module: &mut crate::ir::Module) -> Result<usize, String> {
    use crate::ir::CellKind;
    use crate::ir::emit::{BitView, SigBit};
    let gone: Vec<crate::ir::CellId> = {
        if !module
            .cells
            .iter()
            .any(|(_, c)| matches!(&c.kind, CellKind::Blackbox(p) if p.as_str() == PRIMITIVE))
        {
            return Ok(0);
        }
        let view = BitView::new(module).map_err(|e| e.to_string())?;
        let reads = crate::fpga::srl::read_counts(module, &view);
        module
            .cells
            .iter()
            .filter(
                |(_, cell)| matches!(&cell.kind, CellKind::Blackbox(p) if p.as_str() == PRIMITIVE),
            )
            .filter(|(_, cell)| {
                cell.outputs.iter().all(|(_, net)| {
                    view.net_bits(*net, cell.span).is_ok_and(|bits| {
                        bits.iter().all(|bit| {
                            let bit: SigBit = view.canonical(*bit);
                            reads.get(&bit).copied().unwrap_or(0) == 0
                        })
                    })
                })
            })
            .map(|(id, _)| id)
            .collect()
    };
    module.cells.retain(|id, _| !gone.contains(&id));
    Ok(gone.len())
}
