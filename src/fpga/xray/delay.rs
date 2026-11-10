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
//! *And `DMUX` too, read since*: the one timed pip into it, a carry's
//! `COUT ->> DMUX`, made it cost 0.19 ns whatever left by it, where Vivado
//! charges that only a carry's last carry out; it is charged at that carry's
//! sinks instead (`sites::cout_exit`).
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
//! *Since, the branches are charged otherwise* ([`branch_load`]): one figure a
//! branch was a line through a curve, a wire's second branch costing less than
//! it and each after about twice as much, and a connection of a signal of more
//! than 128 sinks, whose wires have ten or twenty branches, came out 0.045 ns
//! under Vivado's figure. The classes are as they were fitted, beside
//! [`BRANCH_PS`].
//!
//! [`DelayModel::Legacy`] is the model before, prjxray's pip delays alone, and
//! every build of the ChipWhisperer-Husky that was measured on a board was
//! placed and routed with it.
//!
//! # Speed grades
//!
//! **The table has a figure for each class at each of grades 1, 2 and 3**, and
//! the branch charge and the share of prjxray's delay are the grade's too
//! ([`SpeedGrade`]). Grade 3's is the fit above. Grades 1 and 2 are the same
//! 27,016 connections, the same placements and routes, given to Vivado again at
//! `xc7a100tftg256-1` and `-2` and fitted the same way
//! (`tools/vivado-parity/regrade.sh`; `testdata/fpga/vivado/timing/classes_1.txt`
//! and `classes_2.txt`): 1.002 of Vivado's figure at the median at each grade,
//! 0.960 to 1.035 from the 5th to the 95th percentile at grade 1 and 0.955 to
//! 1.041 at grade 2. **Measured.** One factor a grade on grade 3's table would
//! not do as well: the classes do not slow alike — a slice's outputs 1.43 to
//! 1.84 times slower at grade 1, its lookup tables' slow inputs 1.28 to 1.31, a
//! six-tile line 1.27 — and on routes neither had seen, grade 3's table times 1.40
//! was 0.90 to 1.06 of Vivado's grade-1 figure where grade 1's own is 0.93 to
//! 1.04 (`docs/fpga-vivado-parity.md`, "Speed grades"). A `-2L` part times as
//! `-2` does: Vivado's figures for `xc7a35tftg256-2L` are its `-2`'s, every
//! one, as those of the industrial `xc7a35tiftg256-1L` are its `-1`'s.

/// A 7-series speed grade: which of the fitted model's figures a build times
/// with ([`CLASSES`], [`branch_load`], [`OTHER_PER_MILLE`], and the cells' and the
/// clock's in `xray::timing`). Grade 1 is the slowest. [`DelayModel::Legacy`]
/// has one set of figures for every grade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedGrade {
    /// `-1`: the Basys 3's `xc7a35tcpg236-1`, and both XC7A35T devices
    /// Reticle knows.
    One,
    /// `-2`, and `-2L`, which Vivado times the same.
    Two,
    /// `-3`: the ChipWhisperer-Husky Plus's `xc7a100tftg256-3`, at which the
    /// model was first fitted.
    Three,
}

impl SpeedGrade {
    /// The grade a device file's `speed` or `--speed-grade` names, `1`, `2`,
    /// `2L` or `3` (a leading `-` allowed), or `None` for any other. `1L` is
    /// not one: Vivado has no `-1L` of a part Reticle knows, only of an
    /// industrial die of its own name (`xc7a35tiftg256-1L`), which times as
    /// `-1` does.
    #[must_use]
    pub fn from_name(name: &str) -> Option<SpeedGrade> {
        match name.strip_prefix('-').unwrap_or(name) {
            "1" => Some(SpeedGrade::One),
            "2" | "2L" => Some(SpeedGrade::Two),
            "3" => Some(SpeedGrade::Three),
            _ => None,
        }
    }

    /// The grade's place among figures kept slowest first, as [`CLASSES`]
    /// keeps them: 0 for grade 1, 2 for grade 3.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            SpeedGrade::One => 0,
            SpeedGrade::Two => 1,
            SpeedGrade::Three => 2,
        }
    }

    /// The grade's number as Vivado writes it after the part, without the `-`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SpeedGrade::One => "1",
            SpeedGrade::Two => "2",
            SpeedGrade::Three => "3",
        }
    }
}

/// Which delay model a build places, routes and reports with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DelayModel {
    /// The model fitted to Vivado's timer: this module's table for wires, and
    /// Vivado's cell arcs (`xray::timing`), at the build's [`SpeedGrade`].
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
/// to it, in picoseconds, at grades 1, 2 and 3 ([`SpeedGrade::index`]): the
/// fits' 10.1, 9.1 and 8.4, **the charge [`CLASSES`] was fitted beside**, one
/// figure a branch however many leave the wire.
///
/// The model no longer charges it ([`branch_load`]): the same fit's error, by
/// how many branches leave a wire, is a curve and not a line, and its line
/// put the connections of a signal of more than 128 sinks 0.045 ns under
/// Vivado's figure (`docs/fpga-vivado-parity.md`, "A wide signal's
/// connections"). It is kept for the classes, whose fit it was part of.
pub const BRANCH_PS: [u32; 3] = [10, 9, 8];

/// What the second branch leaving a wire on a connection's way adds to it, in
/// picoseconds, at grades 1, 2 and 3 ([`branch_load`]): the fits' 6.1, 5.0
/// and 4.4.
pub const BRANCH_FIRST_PS: [u32; 3] = [6, 5, 4];

/// What each branch past the second leaving a wire on a connection's way adds
/// to it, in picoseconds, at grades 1, 2 and 3 ([`branch_load`]): the fits'
/// 12.8, 11.9 and 11.2.
pub const BRANCH_MORE_PS: [u32; 3] = [13, 12, 11];

/// **What the branches leaving a wire load a connection through it with** at
/// `grade`: nothing for one, [`BRANCH_FIRST_PS`] for the second and
/// [`BRANCH_MORE_PS`] for each after.
///
/// Fitted with [`CLASSES`] held, to Vivado's interconnect delay of 77,420
/// connections of Reticle's own routes of the QoR corpus (`achieved.sh`'s
/// seventeen designs, seeds 1 to 3), timed again at each grade on the same
/// routes. A figure for each count of branches fitted on its own is 5, 13, 26,
/// 36, 49, 60, 73, 83 and 95 ps at grade 3 for two to ten: twice the line's
/// slope from the third on, and under it for the second. On a signal of more
/// than 128 sinks a connection is 0.001 ns under Vivado's at grade 3 where the
/// classes' own 8.4 ps a branch put it 0.045 under, and from 65 to 128 sinks
/// 0.002 over where it was 0.008 under.
/// **Checked**, against Vivado's numbers, a connection at a time
/// (`testdata/fpga/vivado/timing/branches.txt`); nothing here has been on a
/// part.
#[must_use]
pub fn branch_load(grade: SpeedGrade) -> crate::fpga::route::BranchLoad {
    crate::fpga::route::BranchLoad {
        first: BRANCH_FIRST_PS[grade.index()],
        more: BRANCH_MORE_PS[grade.index()],
    }
}

/// What a wire of no class in [`CLASSES`] costs, in thousandths of prjxray's
/// delay for the pip that enters it, at grades 1, 2 and 3.
pub const OTHER_PER_MILLE: [u32; 3] = [939, 726, 631];

/// [`BRANCH_PS`] at `grade`: the classes' fit's charge a branch, not the
/// model's ([`branch_load`]).
#[must_use]
pub fn branch_ps(grade: SpeedGrade) -> u32 {
    BRANCH_PS[grade.index()]
}

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

/// What a wire of `class` costs at `grade`, in picoseconds, when the fit gave
/// it one.
#[must_use]
pub fn class_ps(class: &str, grade: SpeedGrade) -> Option<u32> {
    CLASSES
        .binary_search_by(|(name, _)| (*name).cmp(class))
        .ok()
        .map(|i| u32::from(CLASSES[i].1[grade.index()]))
}

/// What a wire costs at `grade` that a pip in its own tile enters, given
/// prjxray's delay for that pip, if it gives one.
#[must_use]
pub fn wire_ps(tile_type: &str, wire: &str, prjxray_ps: Option<u32>, grade: SpeedGrade) -> u32 {
    class_ps(&wire_class(tile_type, wire), grade)
        .unwrap_or_else(|| prjxray_ps.map_or(0, |ps| ps * OTHER_PER_MILLE[grade.index()] / 1000))
}

/// Each class and its fitted cost in picoseconds at grades 1, 2 and 3, sorted
/// by name: `timing_table.py rust` of `testdata/fpga/vivado/timing/`'s
/// `classes_1.txt`, `classes_2.txt` and `classes.txt`.
pub const CLASSES: &[(&str, [u16; 3])] = &[
    ("BRAM:BRAM_ADDRARDADDRL", [75, 63, 57]),
    ("BRAM:BRAM_ADDRARDADDRU", [80, 67, 61]),
    ("BRAM:BRAM_ADDRBWRADDRL", [72, 61, 55]),
    ("BRAM:BRAM_ADDRBWRADDRU", [73, 61, 55]),
    ("BRAM:BRAM_FIFO36_ADDRARDADDRL", [75, 63, 57]),
    ("BRAM:BRAM_FIFO36_ADDRARDADDRU", [80, 67, 61]),
    ("BRAM:BRAM_FIFO36_ADDRBWRADDRL", [72, 61, 55]),
    ("BRAM:BRAM_FIFO36_ADDRBWRADDRU", [73, 61, 55]),
    ("BRAM:BRAM_FIFO36_DIADIL", [72, 66, 62]),
    ("BRAM:BRAM_FIFO36_DIADIU", [78, 71, 66]),
    ("BRAM:BRAM_FIFO36_WEAL", [156, 136, 122]),
    ("BRAM:BRAM_IMUX_ADDRARDADDRL", [45, 36, 31]),
    ("BRAM:BRAM_IMUX_ADDRBWRADDRU", [50, 41, 37]),
    ("BRAM:BRAM_LOGIC_OUTS_B1_", [12, 11, 10]),
    ("BRAM:BRAM_LOGIC_OUTS_B3_", [2, 3, 3]),
    ("BRAM:BRAM_LOGIC_OUTS_B4_", [0, 0, 0]),
    ("BRAM:BRAM_LOGIC_OUTS_B6_", [0, 0, 0]),
    ("BRAM:BRAM_R_IMUX_ADDRARDADDRL", [88, 75, 68]),
    ("BRAM:BRAM_R_IMUX_ADDRARDADDRU", [80, 67, 61]),
    ("BRAM:BRAM_R_IMUX_ADDRBWRADDRL", [72, 61, 55]),
    ("BRAM:BRAM_R_IMUX_ADDRBWRADDRU", [77, 65, 58]),
    (
        "BRAM_INT_INTERFACE:INT_INTERFACE_LOGIC_OUTS",
        [328, 265, 234],
    ),
    ("BRKH_INT:BRKH_INT_NL1BEG0_SLOW", [303, 235, 204]),
    ("BRKH_INT:BRKH_INT_NL1BEG1_SLOW", [285, 220, 193]),
    ("BRKH_INT:BRKH_INT_NR1BEG0_SLOW", [288, 223, 194]),
    ("BRKH_INT:BRKH_INT_NR1BEG1_SLOW", [303, 236, 206]),
    ("BRKH_INT:BRKH_INT_SL1END", [279, 217, 190]),
    ("BRKH_INT:BRKH_INT_SR1END", [285, 218, 189]),
    ("CLBLL:LL_A1", [513, 433, 394]),
    ("CLBLL:LL_A2", [495, 418, 383]),
    ("CLBLL:LL_A3", [345, 281, 246]),
    ("CLBLL:LL_A4", [280, 229, 203]),
    ("CLBLL:LL_A5", [114, 91, 80]),
    ("CLBLL:LL_A6", [1, 0, 0]),
    ("CLBLL:LL_AX", [28, 22, 19]),
    ("CLBLL:LL_B1", [507, 426, 388]),
    ("CLBLL:LL_B2", [499, 425, 391]),
    ("CLBLL:LL_B3", [338, 278, 249]),
    ("CLBLL:LL_B4", [269, 222, 198]),
    ("CLBLL:LL_B5", [122, 102, 90]),
    ("CLBLL:LL_B6", [1, 1, 2]),
    ("CLBLL:LL_BX", [18, 14, 12]),
    ("CLBLL:LL_C1", [510, 432, 395]),
    ("CLBLL:LL_C2", [503, 426, 388]),
    ("CLBLL:LL_C3", [341, 279, 249]),
    ("CLBLL:LL_C4", [277, 229, 204]),
    ("CLBLL:LL_C5", [122, 101, 89]),
    ("CLBLL:LL_C6", [0, 0, 0]),
    ("CLBLL:LL_CE", [14, 10, 9]),
    ("CLBLL:LL_COUT_N", [0, 0, 0]),
    ("CLBLL:LL_CX", [20, 16, 14]),
    ("CLBLL:LL_D1", [513, 433, 394]),
    ("CLBLL:LL_D2", [495, 420, 385]),
    ("CLBLL:LL_D3", [339, 280, 251]),
    ("CLBLL:LL_D4", [268, 221, 198]),
    ("CLBLL:LL_D5", [113, 92, 81]),
    ("CLBLL:LL_D6", [4, 6, 6]),
    ("CLBLL:LL_DX", [24, 19, 16]),
    ("CLBLL:LL_SR", [66, 50, 43]),
    ("CLBLL:LOGIC_OUTS0", [69, 52, 44]),
    ("CLBLL:LOGIC_OUTS1", [76, 60, 52]),
    ("CLBLL:LOGIC_OUTS10", [70, 52, 42]),
    ("CLBLL:LOGIC_OUTS11", [72, 54, 44]),
    ("CLBLL:LOGIC_OUTS12", [70, 52, 43]),
    ("CLBLL:LOGIC_OUTS13", [73, 55, 45]),
    ("CLBLL:LOGIC_OUTS14", [72, 53, 44]),
    ("CLBLL:LOGIC_OUTS15", [61, 43, 33]),
    ("CLBLL:LOGIC_OUTS16", [84, 64, 53]),
    ("CLBLL:LOGIC_OUTS17", [79, 59, 48]),
    ("CLBLL:LOGIC_OUTS18", [78, 58, 47]),
    ("CLBLL:LOGIC_OUTS19", [74, 54, 43]),
    ("CLBLL:LOGIC_OUTS2", [66, 49, 42]),
    ("CLBLL:LOGIC_OUTS20", [76, 56, 46]),
    ("CLBLL:LOGIC_OUTS21", [86, 65, 53]),
    ("CLBLL:LOGIC_OUTS22", [82, 62, 51]),
    ("CLBLL:LOGIC_OUTS23", [88, 67, 55]),
    ("CLBLL:LOGIC_OUTS3", [68, 52, 45]),
    ("CLBLL:LOGIC_OUTS4", [76, 60, 53]),
    ("CLBLL:LOGIC_OUTS5", [63, 47, 40]),
    ("CLBLL:LOGIC_OUTS6", [68, 52, 44]),
    ("CLBLL:LOGIC_OUTS7", [72, 56, 49]),
    ("CLBLL:LOGIC_OUTS8", [78, 59, 49]),
    ("CLBLL:LOGIC_OUTS9", [73, 55, 46]),
    ("CLBLL:L_A1", [503, 424, 387]),
    ("CLBLL:L_A2", [490, 413, 379]),
    ("CLBLL:L_A3", [332, 273, 243]),
    ("CLBLL:L_A4", [272, 221, 195]),
    ("CLBLL:L_A5", [102, 82, 73]),
    ("CLBLL:L_A6", [0, 0, 0]),
    ("CLBLL:L_AX", [20, 15, 13]),
    ("CLBLL:L_B1", [506, 426, 388]),
    ("CLBLL:L_B2", [492, 417, 383]),
    ("CLBLL:L_B3", [338, 278, 249]),
    ("CLBLL:L_B4", [274, 226, 202]),
    ("CLBLL:L_B5", [106, 86, 75]),
    ("CLBLL:L_B6", [0, 0, 0]),
    ("CLBLL:L_BX", [19, 14, 12]),
    ("CLBLL:L_C1", [507, 428, 390]),
    ("CLBLL:L_C2", [495, 420, 384]),
    ("CLBLL:L_C3", [339, 277, 247]),
    ("CLBLL:L_C4", [268, 220, 195]),
    ("CLBLL:L_C5", [102, 83, 73]),
    ("CLBLL:L_C6", [0, 0, 0]),
    ("CLBLL:L_CE", [10, 7, 5]),
    ("CLBLL:L_COUT_N", [0, 0, 0]),
    ("CLBLL:L_CX", [19, 15, 13]),
    ("CLBLL:L_D1", [510, 430, 392]),
    ("CLBLL:L_D2", [492, 417, 382]),
    ("CLBLL:L_D3", [335, 276, 247]),
    ("CLBLL:L_D4", [271, 223, 199]),
    ("CLBLL:L_D5", [99, 81, 71]),
    ("CLBLL:L_D6", [0, 0, 0]),
    ("CLBLL:L_DX", [18, 13, 10]),
    ("CLBLL:L_SR", [63, 47, 40]),
    ("CLBLM:LOGIC_OUTS0", [72, 55, 47]),
    ("CLBLM:LOGIC_OUTS1", [69, 53, 45]),
    ("CLBLM:LOGIC_OUTS10", [77, 58, 48]),
    ("CLBLM:LOGIC_OUTS11", [69, 51, 41]),
    ("CLBLM:LOGIC_OUTS12", [82, 63, 53]),
    ("CLBLM:LOGIC_OUTS13", [76, 57, 47]),
    ("CLBLM:LOGIC_OUTS14", [79, 60, 50]),
    ("CLBLM:LOGIC_OUTS15", [80, 61, 51]),
    ("CLBLM:LOGIC_OUTS16", [76, 56, 46]),
    ("CLBLM:LOGIC_OUTS17", [83, 62, 51]),
    ("CLBLM:LOGIC_OUTS18", [71, 52, 42]),
    ("CLBLM:LOGIC_OUTS19", [82, 61, 49]),
    ("CLBLM:LOGIC_OUTS2", [75, 58, 50]),
    ("CLBLM:LOGIC_OUTS20", [90, 68, 56]),
    ("CLBLM:LOGIC_OUTS21", [85, 63, 51]),
    ("CLBLM:LOGIC_OUTS22", [86, 63, 50]),
    ("CLBLM:LOGIC_OUTS23", [86, 64, 52]),
    ("CLBLM:LOGIC_OUTS3", [78, 61, 52]),
    ("CLBLM:LOGIC_OUTS4", [69, 53, 46]),
    ("CLBLM:LOGIC_OUTS5", [65, 49, 41]),
    ("CLBLM:LOGIC_OUTS6", [70, 53, 45]),
    ("CLBLM:LOGIC_OUTS7", [69, 53, 45]),
    ("CLBLM:LOGIC_OUTS8", [59, 42, 34]),
    ("CLBLM:LOGIC_OUTS9", [66, 49, 40]),
    ("CLBLM:L_A1", [509, 430, 392]),
    ("CLBLM:L_A2", [498, 421, 386]),
    ("CLBLM:L_A3", [336, 276, 247]),
    ("CLBLM:L_A4", [273, 222, 197]),
    ("CLBLM:L_A5", [104, 84, 75]),
    ("CLBLM:L_A6", [0, 0, 0]),
    ("CLBLM:L_AX", [20, 16, 13]),
    ("CLBLM:L_B1", [511, 430, 391]),
    ("CLBLM:L_B2", [498, 421, 386]),
    ("CLBLM:L_B3", [342, 282, 253]),
    ("CLBLM:L_B4", [275, 226, 202]),
    ("CLBLM:L_B5", [103, 84, 73]),
    ("CLBLM:L_B6", [0, 0, 0]),
    ("CLBLM:L_BX", [24, 19, 16]),
    ("CLBLM:L_C1", [505, 426, 388]),
    ("CLBLM:L_C2", [493, 419, 383]),
    ("CLBLM:L_C3", [341, 279, 249]),
    ("CLBLM:L_C4", [274, 224, 200]),
    ("CLBLM:L_C5", [101, 82, 73]),
    ("CLBLM:L_C6", [0, 0, 0]),
    ("CLBLM:L_CE", [12, 8, 7]),
    ("CLBLM:L_COUT_N", [0, 0, 0]),
    ("CLBLM:L_CX", [22, 17, 15]),
    ("CLBLM:L_D1", [513, 433, 394]),
    ("CLBLM:L_D2", [493, 419, 383]),
    ("CLBLM:L_D3", [335, 276, 248]),
    ("CLBLM:L_D4", [269, 221, 198]),
    ("CLBLM:L_D5", [101, 82, 73]),
    ("CLBLM:L_D6", [0, 0, 0]),
    ("CLBLM:L_DX", [24, 18, 16]),
    ("CLBLM:L_SR", [56, 41, 35]),
    ("CLBLM:M_A1", [518, 438, 400]),
    ("CLBLM:M_A2", [502, 426, 389]),
    ("CLBLM:M_A3", [362, 295, 262]),
    ("CLBLM:M_A4", [294, 242, 216]),
    ("CLBLM:M_A5", [116, 95, 84]),
    ("CLBLM:M_A6", [0, 0, 0]),
    ("CLBLM:M_AI", [11, 8, 8]),
    ("CLBLM:M_AX", [23, 18, 16]),
    ("CLBLM:M_B1", [510, 430, 393]),
    ("CLBLM:M_B2", [496, 421, 387]),
    ("CLBLM:M_B3", [361, 297, 265]),
    ("CLBLM:M_B4", [290, 238, 212]),
    ("CLBLM:M_B5", [127, 102, 90]),
    ("CLBLM:M_B6", [0, 0, 0]),
    ("CLBLM:M_BI", [16, 14, 13]),
    ("CLBLM:M_BX", [22, 17, 15]),
    ("CLBLM:M_C1", [511, 432, 395]),
    ("CLBLM:M_C2", [492, 418, 384]),
    ("CLBLM:M_C3", [361, 297, 264]),
    ("CLBLM:M_C4", [288, 236, 210]),
    ("CLBLM:M_C5", [121, 99, 89]),
    ("CLBLM:M_C6", [0, 0, 0]),
    ("CLBLM:M_CE", [28, 23, 21]),
    ("CLBLM:M_CI", [29, 26, 24]),
    ("CLBLM:M_COUT_N", [0, 0, 0]),
    ("CLBLM:M_CX", [23, 18, 16]),
    ("CLBLM:M_D1", [532, 447, 408]),
    ("CLBLM:M_D2", [506, 429, 394]),
    ("CLBLM:M_D3", [373, 307, 274]),
    ("CLBLM:M_D4", [301, 248, 222]),
    ("CLBLM:M_D5", [131, 108, 98]),
    ("CLBLM:M_D6", [11, 8, 7]),
    ("CLBLM:M_DI", [24, 21, 20]),
    ("CLBLM:M_DX", [22, 17, 14]),
    ("CLBLM:M_SR", [61, 46, 39]),
    ("DSP:DSP_0_A", [92, 80, 71]),
    ("DSP:DSP_1_A", [89, 77, 69]),
    ("DSP:DSP_1_B", [80, 70, 62]),
    ("DSP:DSP_1_PCIN", [2, 2, 2]),
    ("DSP:DSP_LOGIC_OUTS_B1_", [0, 0, 1]),
    ("DSP:DSP_LOGIC_OUTS_B3_", [0, 0, 0]),
    ("DSP:DSP_LOGIC_OUTS_B4_", [7, 6, 5]),
    ("DSP:DSP_LOGIC_OUTS_B6_", [5, 4, 4]),
    ("DSP:DSP_PCOUT", [2, 2, 2]),
    ("INT:BYP", [22, 17, 15]),
    ("INT:BYP_ALT", [72, 54, 45]),
    ("INT:BYP_BOUNCE", [117, 88, 74]),
    ("INT:CTRL", [61, 45, 39]),
    ("INT:EE2BEG", [160, 130, 120]),
    ("INT:EE4BEG", [199, 160, 142]),
    ("INT:EL1BEG", [142, 121, 105]),
    ("INT:EL1BEG_N", [146, 124, 108]),
    ("INT:ER1BEG", [145, 124, 110]),
    ("INT:ER1BEG_S", [142, 120, 106]),
    ("INT:FAN", [18, 15, 13]),
    ("INT:FAN_ALT", [88, 66, 56]),
    ("INT:FAN_BOUNCE", [102, 77, 64]),
    ("INT:GFAN", [180, 141, 122]),
    ("INT:IMUX", [90, 69, 58]),
    ("INT:LH", [327, 264, 236]),
    ("INT:LV", [318, 251, 225]),
    ("INT:LVB", [322, 260, 222]),
    ("INT:NE2BEG", [155, 131, 113]),
    ("INT:NE6BEG", [220, 185, 170]),
    ("INT:NL1BEG", [139, 108, 95]),
    ("INT:NL1BEG_N", [139, 108, 95]),
    ("INT:NN2BEG", [140, 118, 105]),
    ("INT:NN6BEG", [216, 184, 170]),
    ("INT:NR1BEG", [143, 113, 101]),
    ("INT:NW2BEG", [159, 132, 115]),
    ("INT:NW6BEG", [205, 171, 156]),
    ("INT:SE2BEG", [155, 129, 112]),
    ("INT:SE6BEG", [212, 178, 164]),
    ("INT:SL1BEG", [140, 108, 96]),
    ("INT:SR1BEG", [150, 118, 104]),
    ("INT:SR1BEG_S", [138, 108, 96]),
    ("INT:SS2BEG", [143, 121, 108]),
    ("INT:SS6BEG", [214, 181, 168]),
    ("INT:SW2BEG", [158, 134, 116]),
    ("INT:SW6BEG", [204, 172, 158]),
    ("INT:WL1BEG", [148, 125, 108]),
    ("INT:WL1BEG_N", [145, 121, 106]),
    ("INT:WR1BEG", [147, 125, 110]),
    ("INT:WR1BEG_S", [148, 126, 110]),
    ("INT:WW2BEG", [166, 135, 124]),
    ("INT:WW4BEG", [206, 167, 148]),
    ("INT_INTERFACE:INT_INTERFACE_LOGIC_OUTS", [261, 202, 174]),
    ("IOI3:IOI_ILOGIC0_O", [98, 79, 71]),
    ("IOI3:IOI_ILOGIC1_O", [109, 89, 79]),
    ("IOI3:IOI_LOGIC_OUTS18_", [104, 84, 75]),
    ("IOI3:IOI_OLOGIC0_D", [410, 341, 296]),
    ("IOI3:IOI_OLOGIC1_D", [436, 366, 319]),
    ("IOI3:LIOI_I", [90, 70, 62]),
    ("IOI3:LIOI_ILOGIC0_D", [78, 61, 54]),
    ("IOI3:LIOI_ILOGIC1_D", [95, 74, 65]),
    ("IOI3:LIOI_O", [405, 336, 291]),
    ("IOI3:LIOI_OLOGIC0_OQ", [387, 323, 280]),
    ("IOI3:LIOI_OLOGIC1_OQ", [393, 325, 280]),
    ("IOI3:RIOI_I", [112, 93, 83]),
    ("IOI3:RIOI_ILOGIC0_D", [105, 86, 77]),
    ("IOI3:RIOI_ILOGIC1_D", [120, 101, 91]),
    ("IOI3:RIOI_O", [428, 359, 313]),
    ("IOI3:RIOI_OLOGIC0_OQ", [406, 338, 293]),
    ("IOI3:RIOI_OLOGIC1_OQ", [445, 378, 333]),
    ("IO_INT_INTERFACE:INT_INTERFACE_LOGIC_OUTS", [104, 84, 75]),
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
            for (grade, ps) in GRADES.into_iter().zip(ps) {
                assert_eq!(class_ps(name, grade), Some(u32::from(*ps)), "{name}");
            }
        }
        assert_eq!(class_ps("INT:NO_SUCH_WIRE", SpeedGrade::Three), None);
    }

    const GRADES: [SpeedGrade; 3] = [SpeedGrade::One, SpeedGrade::Two, SpeedGrade::Three];

    /// **A grade is read as Vivado writes it, a grade it has no part of is
    /// refused, and each grade reads its own column.** `2L` is `2`, with or
    /// without the dash; `1L`, `4` and nothing are none. Grade 1 is the
    /// slowest of every class of 20 ps or more and grade 3 the fastest, so a
    /// table whose columns were put in the wrong order would time a grade-1
    /// part as a grade-3 one, and fail here. The branches' load reads its
    /// grade's figures too. Would catch the columns swapped, `2L` refused, or a grade
    /// looked up in another's column, or [`branch_load`] given another grade's
    /// figures or its two swapped; would not catch a figure itself wrong,
    /// which `tests/fpga_xray_delay.rs` holds to Vivado's.
    #[test]
    fn a_grade_reads_its_own_column() {
        assert_eq!(SpeedGrade::from_name("1"), Some(SpeedGrade::One));
        assert_eq!(SpeedGrade::from_name("-2L"), Some(SpeedGrade::Two));
        assert_eq!(SpeedGrade::from_name("2"), Some(SpeedGrade::Two));
        assert_eq!(SpeedGrade::from_name("3"), Some(SpeedGrade::Three));
        for refused in ["1L", "4", "", "C8/I7"] {
            assert_eq!(SpeedGrade::from_name(refused), None, "{refused}");
        }
        for grade in GRADES {
            assert_eq!(SpeedGrade::from_name(grade.name()), Some(grade));
        }
        // A class of a few picoseconds is the fit's noise at any grade (a
        // slice's `B6`, 1, 1 and 2 ps), and is not held to it.
        for (name, ps) in CLASSES.iter().filter(|(_, ps)| ps[2] >= 20) {
            assert!(ps[0] > ps[1] && ps[1] > ps[2], "{name}: {ps:?}");
        }
        assert!(BRANCH_PS[0] > BRANCH_PS[1] && BRANCH_PS[1] > BRANCH_PS[2]);
        assert!(OTHER_PER_MILLE[0] > OTHER_PER_MILLE[1]);
        assert!(OTHER_PER_MILLE[1] > OTHER_PER_MILLE[2]);
        let a1 = |grade| class_ps("CLBLL:LL_A1", grade).unwrap();
        assert_eq!(
            [
                a1(SpeedGrade::One),
                a1(SpeedGrade::Two),
                a1(SpeedGrade::Three)
            ],
            [513, 433, 394]
        );
        assert_eq!(branch_ps(SpeedGrade::One), 10);
        assert_eq!(branch_ps(SpeedGrade::Three), 8);
        for table in [BRANCH_FIRST_PS, BRANCH_MORE_PS] {
            assert!(table[0] > table[1] && table[1] > table[2]);
        }
        let load = |grade| branch_load(grade);
        assert_eq!(
            [
                load(SpeedGrade::One).charge(4),
                load(SpeedGrade::Two).charge(4),
                load(SpeedGrade::Three).charge(4)
            ],
            [6 + 2 * 13, 5 + 2 * 12, 4 + 2 * 11]
        );
    }

    /// **A wire of no class costs a share of prjxray's figure, and nothing when
    /// prjxray has none; one of a class costs the class's whatever prjxray says.**
    /// Would catch a slice's own wires charged as an untimed pip again, which
    /// made a table's output through its multiplexer 0.11 ns dear, the share
    /// not applied, or prjxray's figure taking a class's place.
    #[test]
    fn a_wire_of_no_class_costs_a_share_of_prjxrays_figure() {
        for grade in GRADES {
            assert_eq!(wire_ps("CLBLL_L", "CLBLL_L_MUX_AFF_D", None, grade), 0);
            assert_eq!(
                wire_ps("INT_L", "NO_SUCH_WIRE0", Some(1000), grade),
                OTHER_PER_MILLE[grade.index()]
            );
            let imux = class_ps("INT:IMUX", grade).unwrap();
            assert_eq!(wire_ps("INT_L", "IMUX_L3", Some(1000), grade), imux);
            assert_eq!(wire_ps("INT_L", "IMUX_L3", None, grade), imux);
        }
    }
}
