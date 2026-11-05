//! The 7-series delay model fitted to Vivado's timer, and the choice between
//! it and the one before.
//!
//! # What was measured
//!
//! Vivado 2026.1 timed Reticle's own placed and routed implementations of the
//! designs of `testdata/fpga/vivado/qor/` and a few larger ones, at
//! `xc7a100tftg256-3`, the Husky Plus's part and grade, with every route fixed
//! as Reticle made it (`tools/vivado-parity/run_placed.sh` with `TIMING=1`).
//! `timing_dump.tcl` asked it, for every connection, the interconnect delay to
//! the sink (`get_net_delays -interconnect_only`, the slow corner's maximum),
//! the pips of the net, and every cell's timing arcs; `timing_table.py fit`
//! fitted the table below to those numbers, and
//! `testdata/fpga/vivado/timing/` keeps them. Only numbers are kept: nothing of
//! Vivado's models.
//!
//! # What the model is
//!
//! **A connection costs the sum of what each wire it enters costs, and a little
//! for every branch it passes.** A wire's cost is looked up by its class
//! ([`wire_class`]): the tile type without its side, and the wire's name without
//! its track number — every `SL1BEG` of an interconnect tile is one class, and
//! every `IMUX` another. A slice's own pins keep their number, because they are
//! not alike: `A6` is the fastest input of a lookup table and `A1` the slowest,
//! and Vivado puts that difference in the wire to the pin, not in the table's
//! arc. A join between the pieces of one wire in two tiles costs nothing, since
//! the wire was paid for where it was entered. A class the fit did not see often
//! enough to have an entry costs [`OTHER_PER_MILLE`] of prjxray's delay for the
//! pip ([`super::XrayDatabase::node_delays`]), and nothing when prjxray gives
//! the pip none: those are this flow's own wires inside a slice (`_MUX_`,
//! `_CARRY_`), whose delay Vivado puts in the cell's arc and not in a wire.
//! Charged as the legacy model's untimed pip, they made a lookup table's
//! output through its slice's multiplexer 0.11 ns dearer than Vivado has it.
//!
//! Each branch more than one leaving a wire on a connection's way adds
//! [`BRANCH_PS`]: the load of the other branches, which Vivado charges and a sum
//! of wires does not see. Without it, a sink of a signal with many is
//! overstated, and one with few understated.
//!
//! Over the 27,016 connections it was fitted to, the model's figure over
//! Vivado's is 1.002 at the median, 0.952 at the 5th percentile and 1.044 at the
//! 95th. Fitted on half the designs it is 0.92 to 1.07 on the other half.
//! **Checked**, against Vivado's numbers, a connection at a time; nothing here
//! has been on a part.
//!
//! [`DelayModel::Legacy`] is the model before, prjxray's pip delays alone, and
//! every build of the ChipWhisperer-Husky that was measured on a board was
//! placed and routed with it.

/// Which delay model a build places, routes and reports with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DelayModel {
    /// The model fitted to Vivado's timer: this module's table for wires, and
    /// Vivado's cell arcs at grade 3 (`xray::timing`).
    #[default]
    Vivado,
    /// prjxray's pip delays and the cell figures from before the fit, which
    /// every measured Husky build used.
    Legacy,
}

impl DelayModel {
    /// The model `--delay-model` names, or `None` for a name that is neither.
    #[must_use]
    pub fn from_name(name: &str) -> Option<DelayModel> {
        match name {
            "vivado" => Some(DelayModel::Vivado),
            "legacy" => Some(DelayModel::Legacy),
            _ => None,
        }
    }
}

/// What each branch more than one leaving a wire on a connection's way adds
/// to it, in picoseconds.
pub const BRANCH_PS: u32 = 8;

/// What a wire of no class in [`CLASSES`] costs, in thousandths of prjxray's
/// delay for the pip that enters it.
pub const OTHER_PER_MILLE: u32 = 631;

/// A tile type without its side or its bit alias: `INT_L` and `INT_R` are
/// `INT`, `CLBLM_L` is `CLBLM`, `RIOI3_TBYTESRC` is `IOI3`, and
/// `BRAM_INT_INTERFACE_R` is `BRAM_INT_INTERFACE`.
#[must_use]
pub fn family(tile_type: &str) -> &str {
    // A tile type split by bit alias is `<type>_W<offset>`.
    let t = tile_type
        .rsplit_once("_W")
        .filter(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .map_or(tile_type, |(head, _)| head);
    if let Some(rest) = t.strip_prefix('L').or_else(|| t.strip_prefix('R'))
        && (rest.starts_with("IOI") || rest.starts_with("IOB"))
    {
        let digits = rest[3..].bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            return &rest[..3 + digits];
        }
    }
    for base in ["CLBLL", "CLBLM", "BRAM", "DSP", "INT"] {
        if t.len() == base.len() + 2
            && t.starts_with(base)
            && (t.ends_with("_L") || t.ends_with("_R"))
        {
            return &t[..base.len()];
        }
    }
    t.strip_suffix("_L")
        .or_else(|| t.strip_suffix("_R"))
        .unwrap_or(t)
}

/// The class a wire is costed by: `<family>:<wire>`, the wire with the `_L` a
/// left tile writes before its number taken out, and, outside a slice's tile,
/// its number too. `INT_L`'s `IMUX_L17` is `INT:IMUX`, and `CLBLL_R`'s
/// `CLBLL_LL_A3` is `CLBLL:LL_A3`.
#[must_use]
pub fn wire_class(tile_type: &str, wire: &str) -> String {
    let fam = family(tile_type);
    let digits = wire.bytes().rev().take_while(u8::is_ascii_digit).count();
    let (stem, number) = wire.split_at(wire.len() - digits);
    let stem = if digits > 0 {
        stem.strip_suffix("_L").unwrap_or(stem)
    } else {
        stem
    };
    if fam == "CLBLL" || fam == "CLBLM" {
        let stem = stem
            .strip_prefix("CLBLL_")
            .or_else(|| stem.strip_prefix("CLBLM_"))
            .unwrap_or(stem);
        format!("{fam}:{stem}{number}")
    } else {
        format!("{fam}:{stem}")
    }
}

/// What a wire of `class` costs, in picoseconds, when the fit gave it one.
#[must_use]
pub fn class_ps(class: &str) -> Option<u32> {
    CLASSES
        .binary_search_by(|(name, _)| (*name).cmp(class))
        .ok()
        .map(|i| u32::from(CLASSES[i].1))
}

/// What a wire costs that a pip in its own tile enters, given prjxray's delay
/// for that pip, if it gives one.
#[must_use]
pub fn wire_ps(tile_type: &str, wire: &str, prjxray_ps: Option<u32>) -> u32 {
    class_ps(&wire_class(tile_type, wire))
        .unwrap_or_else(|| prjxray_ps.map_or(0, |ps| ps * OTHER_PER_MILLE / 1000))
}

/// Each class and its fitted cost in picoseconds, sorted by name:
/// `timing_table.py rust testdata/fpga/vivado/timing/classes.txt`.
pub const CLASSES: &[(&str, u16)] = &[
    ("BRAM:BRAM_ADDRARDADDRL", 57),
    ("BRAM:BRAM_ADDRARDADDRU", 61),
    ("BRAM:BRAM_ADDRBWRADDRL", 55),
    ("BRAM:BRAM_ADDRBWRADDRU", 55),
    ("BRAM:BRAM_FIFO36_ADDRARDADDRL", 57),
    ("BRAM:BRAM_FIFO36_ADDRARDADDRU", 61),
    ("BRAM:BRAM_FIFO36_ADDRBWRADDRL", 55),
    ("BRAM:BRAM_FIFO36_ADDRBWRADDRU", 55),
    ("BRAM:BRAM_FIFO36_DIADIL", 62),
    ("BRAM:BRAM_FIFO36_DIADIU", 66),
    ("BRAM:BRAM_FIFO36_WEAL", 122),
    ("BRAM:BRAM_IMUX_ADDRARDADDRL", 31),
    ("BRAM:BRAM_IMUX_ADDRBWRADDRU", 37),
    ("BRAM:BRAM_LOGIC_OUTS_B1_", 10),
    ("BRAM:BRAM_LOGIC_OUTS_B3_", 3),
    ("BRAM:BRAM_LOGIC_OUTS_B4_", 0),
    ("BRAM:BRAM_LOGIC_OUTS_B6_", 0),
    ("BRAM:BRAM_R_IMUX_ADDRARDADDRL", 68),
    ("BRAM:BRAM_R_IMUX_ADDRARDADDRU", 61),
    ("BRAM:BRAM_R_IMUX_ADDRBWRADDRL", 55),
    ("BRAM:BRAM_R_IMUX_ADDRBWRADDRU", 58),
    ("BRAM_INT_INTERFACE:INT_INTERFACE_LOGIC_OUTS", 234),
    ("BRKH_INT:BRKH_INT_NL1BEG0_SLOW", 204),
    ("BRKH_INT:BRKH_INT_NL1BEG1_SLOW", 193),
    ("BRKH_INT:BRKH_INT_NR1BEG0_SLOW", 194),
    ("BRKH_INT:BRKH_INT_NR1BEG1_SLOW", 206),
    ("BRKH_INT:BRKH_INT_SL1END", 190),
    ("BRKH_INT:BRKH_INT_SR1END", 189),
    ("CLBLL:LL_A1", 394),
    ("CLBLL:LL_A2", 383),
    ("CLBLL:LL_A3", 246),
    ("CLBLL:LL_A4", 203),
    ("CLBLL:LL_A5", 80),
    ("CLBLL:LL_A6", 0),
    ("CLBLL:LL_AX", 19),
    ("CLBLL:LL_B1", 388),
    ("CLBLL:LL_B2", 391),
    ("CLBLL:LL_B3", 249),
    ("CLBLL:LL_B4", 198),
    ("CLBLL:LL_B5", 90),
    ("CLBLL:LL_B6", 2),
    ("CLBLL:LL_BX", 12),
    ("CLBLL:LL_C1", 395),
    ("CLBLL:LL_C2", 388),
    ("CLBLL:LL_C3", 249),
    ("CLBLL:LL_C4", 204),
    ("CLBLL:LL_C5", 89),
    ("CLBLL:LL_C6", 0),
    ("CLBLL:LL_CE", 9),
    ("CLBLL:LL_COUT_N", 0),
    ("CLBLL:LL_CX", 14),
    ("CLBLL:LL_D1", 394),
    ("CLBLL:LL_D2", 385),
    ("CLBLL:LL_D3", 251),
    ("CLBLL:LL_D4", 198),
    ("CLBLL:LL_D5", 81),
    ("CLBLL:LL_D6", 6),
    ("CLBLL:LL_DX", 16),
    ("CLBLL:LL_SR", 43),
    ("CLBLL:LOGIC_OUTS0", 44),
    ("CLBLL:LOGIC_OUTS1", 52),
    ("CLBLL:LOGIC_OUTS10", 42),
    ("CLBLL:LOGIC_OUTS11", 44),
    ("CLBLL:LOGIC_OUTS12", 43),
    ("CLBLL:LOGIC_OUTS13", 45),
    ("CLBLL:LOGIC_OUTS14", 44),
    ("CLBLL:LOGIC_OUTS15", 33),
    ("CLBLL:LOGIC_OUTS16", 53),
    ("CLBLL:LOGIC_OUTS17", 48),
    ("CLBLL:LOGIC_OUTS18", 47),
    ("CLBLL:LOGIC_OUTS19", 43),
    ("CLBLL:LOGIC_OUTS2", 42),
    ("CLBLL:LOGIC_OUTS20", 46),
    ("CLBLL:LOGIC_OUTS21", 53),
    ("CLBLL:LOGIC_OUTS22", 51),
    ("CLBLL:LOGIC_OUTS23", 55),
    ("CLBLL:LOGIC_OUTS3", 45),
    ("CLBLL:LOGIC_OUTS4", 53),
    ("CLBLL:LOGIC_OUTS5", 40),
    ("CLBLL:LOGIC_OUTS6", 44),
    ("CLBLL:LOGIC_OUTS7", 49),
    ("CLBLL:LOGIC_OUTS8", 49),
    ("CLBLL:LOGIC_OUTS9", 46),
    ("CLBLL:L_A1", 387),
    ("CLBLL:L_A2", 379),
    ("CLBLL:L_A3", 243),
    ("CLBLL:L_A4", 195),
    ("CLBLL:L_A5", 73),
    ("CLBLL:L_A6", 0),
    ("CLBLL:L_AX", 13),
    ("CLBLL:L_B1", 388),
    ("CLBLL:L_B2", 383),
    ("CLBLL:L_B3", 249),
    ("CLBLL:L_B4", 202),
    ("CLBLL:L_B5", 75),
    ("CLBLL:L_B6", 0),
    ("CLBLL:L_BX", 12),
    ("CLBLL:L_C1", 390),
    ("CLBLL:L_C2", 384),
    ("CLBLL:L_C3", 247),
    ("CLBLL:L_C4", 195),
    ("CLBLL:L_C5", 73),
    ("CLBLL:L_C6", 0),
    ("CLBLL:L_CE", 5),
    ("CLBLL:L_COUT_N", 0),
    ("CLBLL:L_CX", 13),
    ("CLBLL:L_D1", 392),
    ("CLBLL:L_D2", 382),
    ("CLBLL:L_D3", 247),
    ("CLBLL:L_D4", 199),
    ("CLBLL:L_D5", 71),
    ("CLBLL:L_D6", 0),
    ("CLBLL:L_DX", 10),
    ("CLBLL:L_SR", 40),
    ("CLBLM:LOGIC_OUTS0", 47),
    ("CLBLM:LOGIC_OUTS1", 45),
    ("CLBLM:LOGIC_OUTS10", 48),
    ("CLBLM:LOGIC_OUTS11", 41),
    ("CLBLM:LOGIC_OUTS12", 53),
    ("CLBLM:LOGIC_OUTS13", 47),
    ("CLBLM:LOGIC_OUTS14", 50),
    ("CLBLM:LOGIC_OUTS15", 51),
    ("CLBLM:LOGIC_OUTS16", 46),
    ("CLBLM:LOGIC_OUTS17", 51),
    ("CLBLM:LOGIC_OUTS18", 42),
    ("CLBLM:LOGIC_OUTS19", 49),
    ("CLBLM:LOGIC_OUTS2", 50),
    ("CLBLM:LOGIC_OUTS20", 56),
    ("CLBLM:LOGIC_OUTS21", 51),
    ("CLBLM:LOGIC_OUTS22", 50),
    ("CLBLM:LOGIC_OUTS23", 52),
    ("CLBLM:LOGIC_OUTS3", 52),
    ("CLBLM:LOGIC_OUTS4", 46),
    ("CLBLM:LOGIC_OUTS5", 41),
    ("CLBLM:LOGIC_OUTS6", 45),
    ("CLBLM:LOGIC_OUTS7", 45),
    ("CLBLM:LOGIC_OUTS8", 34),
    ("CLBLM:LOGIC_OUTS9", 40),
    ("CLBLM:L_A1", 392),
    ("CLBLM:L_A2", 386),
    ("CLBLM:L_A3", 247),
    ("CLBLM:L_A4", 197),
    ("CLBLM:L_A5", 75),
    ("CLBLM:L_A6", 0),
    ("CLBLM:L_AX", 13),
    ("CLBLM:L_B1", 391),
    ("CLBLM:L_B2", 386),
    ("CLBLM:L_B3", 253),
    ("CLBLM:L_B4", 202),
    ("CLBLM:L_B5", 73),
    ("CLBLM:L_B6", 0),
    ("CLBLM:L_BX", 16),
    ("CLBLM:L_C1", 388),
    ("CLBLM:L_C2", 383),
    ("CLBLM:L_C3", 249),
    ("CLBLM:L_C4", 200),
    ("CLBLM:L_C5", 73),
    ("CLBLM:L_C6", 0),
    ("CLBLM:L_CE", 7),
    ("CLBLM:L_COUT_N", 0),
    ("CLBLM:L_CX", 15),
    ("CLBLM:L_D1", 394),
    ("CLBLM:L_D2", 383),
    ("CLBLM:L_D3", 248),
    ("CLBLM:L_D4", 198),
    ("CLBLM:L_D5", 73),
    ("CLBLM:L_D6", 0),
    ("CLBLM:L_DX", 16),
    ("CLBLM:L_SR", 35),
    ("CLBLM:M_A1", 400),
    ("CLBLM:M_A2", 389),
    ("CLBLM:M_A3", 262),
    ("CLBLM:M_A4", 216),
    ("CLBLM:M_A5", 84),
    ("CLBLM:M_A6", 0),
    ("CLBLM:M_AI", 8),
    ("CLBLM:M_AX", 16),
    ("CLBLM:M_B1", 393),
    ("CLBLM:M_B2", 387),
    ("CLBLM:M_B3", 265),
    ("CLBLM:M_B4", 212),
    ("CLBLM:M_B5", 90),
    ("CLBLM:M_B6", 0),
    ("CLBLM:M_BI", 13),
    ("CLBLM:M_BX", 15),
    ("CLBLM:M_C1", 395),
    ("CLBLM:M_C2", 384),
    ("CLBLM:M_C3", 264),
    ("CLBLM:M_C4", 210),
    ("CLBLM:M_C5", 89),
    ("CLBLM:M_C6", 0),
    ("CLBLM:M_CE", 21),
    ("CLBLM:M_CI", 24),
    ("CLBLM:M_COUT_N", 0),
    ("CLBLM:M_CX", 16),
    ("CLBLM:M_D1", 408),
    ("CLBLM:M_D2", 394),
    ("CLBLM:M_D3", 274),
    ("CLBLM:M_D4", 222),
    ("CLBLM:M_D5", 98),
    ("CLBLM:M_D6", 7),
    ("CLBLM:M_DI", 20),
    ("CLBLM:M_DX", 14),
    ("CLBLM:M_SR", 39),
    ("DSP:DSP_0_A", 71),
    ("DSP:DSP_1_A", 69),
    ("DSP:DSP_1_B", 62),
    ("DSP:DSP_1_PCIN", 2),
    ("DSP:DSP_LOGIC_OUTS_B1_", 1),
    ("DSP:DSP_LOGIC_OUTS_B3_", 0),
    ("DSP:DSP_LOGIC_OUTS_B4_", 5),
    ("DSP:DSP_LOGIC_OUTS_B6_", 4),
    ("DSP:DSP_PCOUT", 2),
    ("INT:BYP", 15),
    ("INT:BYP_ALT", 45),
    ("INT:BYP_BOUNCE", 74),
    ("INT:CTRL", 39),
    ("INT:EE2BEG", 120),
    ("INT:EE4BEG", 142),
    ("INT:EL1BEG", 105),
    ("INT:EL1BEG_N", 108),
    ("INT:ER1BEG", 110),
    ("INT:ER1BEG_S", 106),
    ("INT:FAN", 13),
    ("INT:FAN_ALT", 56),
    ("INT:FAN_BOUNCE", 64),
    ("INT:GFAN", 122),
    ("INT:IMUX", 58),
    ("INT:LH", 236),
    ("INT:LV", 225),
    ("INT:LVB", 222),
    ("INT:NE2BEG", 113),
    ("INT:NE6BEG", 170),
    ("INT:NL1BEG", 95),
    ("INT:NL1BEG_N", 95),
    ("INT:NN2BEG", 105),
    ("INT:NN6BEG", 170),
    ("INT:NR1BEG", 101),
    ("INT:NW2BEG", 115),
    ("INT:NW6BEG", 156),
    ("INT:SE2BEG", 112),
    ("INT:SE6BEG", 164),
    ("INT:SL1BEG", 96),
    ("INT:SR1BEG", 104),
    ("INT:SR1BEG_S", 96),
    ("INT:SS2BEG", 108),
    ("INT:SS6BEG", 168),
    ("INT:SW2BEG", 116),
    ("INT:SW6BEG", 158),
    ("INT:WL1BEG", 108),
    ("INT:WL1BEG_N", 106),
    ("INT:WR1BEG", 110),
    ("INT:WR1BEG_S", 110),
    ("INT:WW2BEG", 124),
    ("INT:WW4BEG", 148),
    ("INT_INTERFACE:INT_INTERFACE_LOGIC_OUTS", 174),
    ("IOI3:IOI_ILOGIC0_O", 71),
    ("IOI3:IOI_ILOGIC1_O", 79),
    ("IOI3:IOI_LOGIC_OUTS18_", 75),
    ("IOI3:IOI_OLOGIC0_D", 296),
    ("IOI3:IOI_OLOGIC1_D", 319),
    ("IOI3:LIOI_I", 62),
    ("IOI3:LIOI_ILOGIC0_D", 54),
    ("IOI3:LIOI_ILOGIC1_D", 65),
    ("IOI3:LIOI_O", 291),
    ("IOI3:LIOI_OLOGIC0_OQ", 280),
    ("IOI3:LIOI_OLOGIC1_OQ", 280),
    ("IOI3:RIOI_I", 83),
    ("IOI3:RIOI_ILOGIC0_D", 77),
    ("IOI3:RIOI_ILOGIC1_D", 91),
    ("IOI3:RIOI_O", 313),
    ("IOI3:RIOI_OLOGIC0_OQ", 293),
    ("IOI3:RIOI_OLOGIC1_OQ", 333),
    ("IO_INT_INTERFACE:INT_INTERFACE_LOGIC_OUTS", 75),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// **A class is what both sides of the die share.** Would catch a family
    /// that kept its side, so that a left tile's wires found no entry, or a
    /// slice pin that lost its number, so that `A1` and `A6` cost the same.
    #[test]
    fn a_class_is_the_same_on_either_side() {
        assert_eq!(wire_class("INT_L", "IMUX_L17"), "INT:IMUX");
        assert_eq!(wire_class("INT_R", "IMUX17"), "INT:IMUX");
        assert_eq!(wire_class("INT_R_W3", "SL1BEG2"), "INT:SL1BEG");
        assert_eq!(wire_class("CLBLL_R", "CLBLL_LL_A3"), "CLBLL:LL_A3");
        assert_eq!(
            wire_class("CLBLM_L", "CLBLM_LOGIC_OUTS7"),
            "CLBLM:LOGIC_OUTS7"
        );
        assert_eq!(wire_class("RIOI3_TBYTESRC", "RIOI_I0"), "IOI3:RIOI_I");
        assert_eq!(
            wire_class("BRAM_INT_INTERFACE_L", "INT_INTERFACE_LOGIC_OUTS_L3"),
            "BRAM_INT_INTERFACE:INT_INTERFACE_LOGIC_OUTS"
        );
        assert_eq!(family("LIOB33_SING"), "IOB33");
        assert_eq!(family("CLK_HROW_BOT_R"), "CLK_HROW_BOT");
    }

    /// **The table is sorted, so that a lookup finds every entry.** Would catch
    /// an entry put in by hand out of order, which `binary_search` would then
    /// miss without a word.
    #[test]
    fn the_table_is_sorted_and_every_entry_is_found() {
        assert!(CLASSES.windows(2).all(|w| w[0].0 < w[1].0));
        for (name, ps) in CLASSES {
            assert_eq!(class_ps(name), Some(u32::from(*ps)), "{name}");
        }
        assert_eq!(class_ps("INT:NO_SUCH_WIRE"), None);
    }

    /// **A wire of no class costs a share of prjxray's figure, and nothing when
    /// prjxray has none; one of a class costs the class's whatever prjxray says.**
    /// Would catch a slice's own wires charged as an untimed pip again, which
    /// made a table's output through its multiplexer 0.11 ns dear, the share
    /// not applied, or prjxray's figure taking a class's place.
    #[test]
    fn a_wire_of_no_class_costs_a_share_of_prjxrays_figure() {
        assert_eq!(wire_ps("CLBLL_L", "CLBLL_L_MUX_AFF_D", None), 0);
        assert_eq!(
            wire_ps("INT_L", "NO_SUCH_WIRE0", Some(1000)),
            OTHER_PER_MILLE
        );
        let imux = class_ps("INT:IMUX").unwrap();
        assert_eq!(wire_ps("INT_L", "IMUX_L3", Some(1000)), imux);
        assert_eq!(wire_ps("INT_L", "IMUX_L3", None), imux);
    }
}
