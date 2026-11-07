//! Reticle's implementation of a design, in the form Vivado reads.
//!
//! Reticle's timing estimate comes from its own model and nothing checks that
//! model, so this writes what it takes for Vivado's timer to look at the same
//! implementation: `tools/vivado-parity/run_placed.tcl` loads it with no
//! synthesis, and `docs/fpga-vivado-parity.md` has what came back.
//!
//! - **a netlist**, structural Verilog that `link_design` reads as it stands:
//!   one bit to a wire, no functions and no assignments;
//! - **a placement**, each cell's site and bel as Vivado spells them, and each
//!   lookup table's `LOCK_PINS`, because which pin a signal arrives on is part
//!   of what was placed and routed;
//! - **routes**, each routed signal's wires as a tree of `<tile>/<wire>` names.
//!
//! # The netlist is rewritten, and why that is safe
//!
//! It is not `--routed-netlist`'s, which is for a simulator and uses functions
//! and assignments that `link_design` refuses. It differs in three ways, each
//! because Vivado would otherwise see a different implementation:
//!
//! - A lookup table's input tied to a constant that its truth table ignores
//!   is **left out**, so a table of two inputs is a `LUT2`. The mapper widens
//!   every table to a `LUT6` and ties what is left to zero; Vivado would route
//!   a ground to each, and Reticle routes nothing there.
//! - A `LUT6_2` is written as its **two halves**, `<cell>__O6` and
//!   `<cell>__O5`, so that each has its own bel and its own pins.
//! - Names are **made safe**: `c<cell>_<name>` and `n<slot>_<name>`, with
//!   anything but a letter, digit or underscore as an underscore. The netlist's
//!   own hold `$`, `.` and `[`, which `get_cells` would read as patterns.
//!
//! # What Reticle's wire names leave out
//!
//! The graph's nodes are tile wires, and the database gives a wire inside a
//! slice no name, so `sites.rs` invents names with `_MUX_` and `_CARRY_` in them
//! to make a mux inside the slice a pip. Vivado routes those itself from the
//! placement. They are dropped here. Joins, which are one Vivado node seen from two tiles,
//! are kept, and the script reduces them when it asks Vivado what each wire's
//! node is.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fmt::Write as _;

use crate::fpga::arch::{NodeId, RoutingGraph};
use crate::fpga::place::{Netlist, Placement};
use crate::fpga::route::{Route, Routing};
use crate::ir::emit::{BitView, SigBit};
use crate::ir::{AttrValue, Bit, Cell, CellKind, Const, Design, ModuleId};

use super::XrayTile;
use super::sites::site_of_prefix;

/// What [`vivado_implementation`] wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VivadoImplementation {
    /// The netlist, structural Verilog for `link_design`.
    pub verilog: String,
    /// A Tcl script that places every cell, fixes it, and locks each lookup
    /// table's pins. It expects the netlist to be linked already.
    pub placement: String,
    /// A Tcl script of one `reticle_route <net> <tree>` for each routed signal,
    /// for the caller to define `reticle_route` and source.
    pub routes: String,
    /// How many cells were placed. A `LUT6_2` counts as two.
    pub cells: usize,
    /// How many routes were written.
    pub nets: usize,
}

/// Why an implementation could not be written for Vivado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VivadoExportError(String);

impl fmt::Display for VivadoExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot write this implementation for Vivado: {}", self.0)
    }
}

impl std::error::Error for VivadoExportError {}

fn fail<T>(message: String) -> Result<T, VivadoExportError> {
    Err(VivadoExportError(message))
}

/// A name with nothing in it that Tcl or a Vivado pattern would read.
fn safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The site and bel of a placed cell, as Vivado spells them, from the bel name
/// the loader gave it (`SLICEL_X0_ALUT`, `RAMB18_Y1`, `BUFGCTRL_X0Y12`).
///
/// `lower` asks for the five-input half of a lookup table, `A5LUT`.
///
/// Returns the site, then the bel. `None` for a bel this does not know how to
/// spell, which the caller reports by name.
fn site_and_bel(
    tile: &XrayTile,
    bel: &str,
    primitive: &str,
    lower: bool,
) -> Option<(String, String)> {
    let site_named = |prefix: &str| site_of_prefix(tile, prefix).map(|(name, _)| name.clone());
    if let Some(rest) = bel.strip_prefix("SLICEL_").or(bel.strip_prefix("SLICEM_")) {
        let (index, sub) = rest.split_once('_')?;
        let site = site_named(&bel[..bel.len() - rest.len() + index.len()])?;
        // A latch is on the flip-flop's element, and a shift register on the
        // six-input table, as `xray::srl` says.
        let name = if let Some(letter) = sub.strip_suffix("LUT") {
            format!("{letter}{}LUT", if lower { 5 } else { 6 })
        } else if let Some(letter) = sub.strip_suffix("LATCH") {
            format!("{letter}FF")
        } else if let Some(letter) = sub
            .strip_suffix("SRL32")
            .or_else(|| sub.strip_suffix("SRL16"))
        {
            format!("{letter}6LUT")
        } else {
            sub.to_owned()
        };
        return Some((site, name));
    }
    if bel.starts_with("RAMB18_") {
        return Some((site_named(bel)?, "RAMB18E1".to_owned()));
    }
    // A FIFO is the lower half's site, or both halves', under a bel of the
    // FIFO's own name (`lutram_probe.tcl`'s report of Vivado's builds:
    // `FIFO18E1.FIFO18E1` on `RAMB18_X3Y38`, `FIFO36E1.FIFO36E1` on
    // `RAMB36_X3Y19`).
    if bel == super::fifo::FIFO18_BEL {
        return Some((site_named("RAMB18_Y0")?, "FIFO18E1".to_owned()));
    }
    if bel == super::fifo::FIFO36_BEL {
        let (site, _) = tile
            .sites
            .iter()
            .find(|(name, _)| name.starts_with("RAMB36_"))?;
        return Some((site.clone(), "FIFO36E1".to_owned()));
    }
    if let Some(block) = bel.strip_prefix("DSP_") {
        // A tile's two blocks, `DSP_0` the lower: see `dsp::bel_name`.
        let block: usize = block.parse().ok()?;
        let mut sites: Vec<&String> = tile
            .sites
            .iter()
            .filter(|(_, kind)| kind == "DSP48E1")
            .map(|(name, _)| name)
            .collect();
        sites.sort_by_key(|name| {
            name.rsplit_once('Y')
                .and_then(|(_, y)| y.parse::<u32>().ok())
        });
        return Some(((*sites.get(block)?).clone(), "DSP48E1".to_owned()));
    }
    // Both halves as one block: the tile's one `RAMB36_X<x>Y<y>` site, which
    // the grid types `RAMBFIFO36E1`.
    if bel == super::bram::RAMB36_BEL {
        let (site, _) = tile
            .sites
            .iter()
            .find(|(name, _)| name.starts_with("RAMB36_"))?;
        return Some((site.clone(), "RAMB36E1".to_owned()));
    }
    if bel.starts_with("IOB_") {
        // Where Vivado 2026.1 put each of its own (`get_property BEL` of each
        // leaf, after `place_design`): an input buffer on `INBUF_EN` and an
        // output on `OUTBUF`, a differential one on the master's. A buffer of
        // both directions, or of two outputs, is a macro of Vivado's that
        // takes its site alone: an empty bel says so.
        let name = match primitive {
            "IBUF" | "IBUFG" | "IBUF_IBUFDISABLE" | "IBUF_INTERMDISABLE" | "IBUFDS" | "IBUFGDS" => {
                "INBUF_EN"
            }
            "OBUF" | "OBUFT" | "OBUFDS" | "OBUFTDS" => "OUTBUF",
            "IOBUF"
            | "IOBUF_INTERMDISABLE"
            | "IOBUFDS"
            | "IOBUFDS_INTERMDISABLE"
            | "IOBUFDS_DIFF_OUT"
            | "IOBUFDS_DIFF_OUT_INTERMDISABLE"
            | "IBUFDS_DIFF_OUT"
            | "IBUFGDS_DIFF_OUT"
            | "IBUFDS_IBUFDISABLE"
            | "IBUFDS_INTERMDISABLE"
            | "IBUFDS_DIFF_OUT_IBUFDISABLE"
            | "IBUFDS_DIFF_OUT_INTERMDISABLE" => "",
            _ => return None,
        };
        return Some((site_named(bel)?, name.to_owned()));
    }
    // An input delay, and the serialisers, which are the IO logic sites'
    // other bels.
    if bel.starts_with("IDELAY_") {
        return Some((site_named(bel)?, "IDELAYE2".to_owned()));
    }
    if bel.starts_with("OLOGIC_") && primitive == "OSERDESE2" {
        return Some((site_named(bel)?, "OSERDESE2".to_owned()));
    }
    if bel.starts_with("ILOGIC_") && primitive == "ISERDESE2" {
        return Some((site_named(bel)?, "ISERDESE2".to_owned()));
    }
    // A clock manager is the one site of its type in its tile.
    if matches!(bel, "MMCME2_ADV" | "PLLE2_ADV") {
        let (site, _) = tile.sites.iter().find(|(_, kind)| kind == bel)?;
        return Some((site.clone(), bel.to_owned()));
    }
    if bel.starts_with("OLOGIC_") {
        return Some((site_named(bel)?, "OUTFF".to_owned()));
    }
    if bel.starts_with("ILOGIC_") {
        return Some((site_named(bel)?, "IFF".to_owned()));
    }
    if let Some(number) = bel.strip_prefix("BUFGCTRL_X0Y") {
        // Tile-local: the sixteen sites of the tile counted upward. See
        // `sites::extra_bels`.
        let number: usize = number.parse().ok()?;
        let mut sites: Vec<&String> = tile.sites.iter().map(|(name, _)| name).collect();
        sites.sort_by_key(|name| {
            name.rsplit_once('Y')
                .and_then(|(_, y)| y.parse::<u32>().ok())
        });
        // Every global buffer but a `BUFG` is a `BUFGCTRL` to Vivado once
        // linked, and goes on that bel: `BUFGCE` on `BUFG` is refused
        // (12-1409, "the bel type (BUFG_BUFG) do not match the cell type
        // (BUFGCTRL)").
        let name = if primitive == "BUFG" {
            "BUFG"
        } else {
            "BUFGCTRL"
        };
        return Some(((*sites.get(number)?).clone(), name.to_owned()));
    }
    // A tile's several sites of a regional clock buffer, by rank up the tile.
    let ranked = |family: &str, column: Option<u32>, number: usize| -> Option<String> {
        let numbers = |name: &str| -> Option<(u32, u32)> {
            let (x, y) = name
                .rsplit_once('_')?
                .1
                .strip_prefix('X')?
                .split_once('Y')?;
            Some((x.parse().ok()?, y.parse().ok()?))
        };
        let mut sites: Vec<(u32, u32, &String)> = tile
            .sites
            .iter()
            .filter(|(_, kind)| kind == family)
            .filter_map(|(name, _)| numbers(name).map(|(x, y)| (x, y, name)))
            .collect();
        sites.sort();
        // A column names the side: its numbers on the die are not its numbers
        // in the tile, which the feature names and `bel` use.
        let first = sites.first()?.0;
        sites
            .iter()
            .filter(|(x, ..)| column.is_none_or(|c| *x == first + c))
            .nth(number)
            .map(|(.., name)| (*name).clone())
    };
    if let Some(rest) = bel.strip_prefix("BUFHCE_X") {
        let (x, y) = rest.split_once('Y')?;
        return Some((
            ranked("BUFHCE", Some(x.parse().ok()?), y.parse().ok()?)?,
            "BUFHCE".to_owned(),
        ));
    }
    if let Some(k) = bel.strip_prefix("BUFMRCE_Y") {
        return Some((
            ranked("BUFMRCE", None, k.parse().ok()?)?,
            "BUFMRCE".to_owned(),
        ));
    }
    for family in ["BUFR", "BUFIO"] {
        if let Some(k) = bel.strip_prefix(family).and_then(|k| k.strip_prefix("_Y")) {
            return Some((ranked(family, None, k.parse().ok()?)?, family.to_owned()));
        }
    }
    // A site the tile has one of, or names exactly: the configuration
    // primitives, the controller of the input delays, the XADC. Vivado's bel
    // is the site's type (`BSCAN_X0Y0` is `BSCAN.BSCAN`, `ICAP_X0Y1`
    // `ICAP.ICAP`, `STARTUP_X0Y0` `STARTUP.STARTUP`).
    if let Some((site, kind)) = tile.sites.iter().find(|(name, _)| name == bel) {
        return Some((site.clone(), kind.clone()));
    }
    // `DNA_PORT` and `EFUSE_USR` are declared on the configuration tile with
    // their pins ten rows up (`config::far_pins`), and their sites are that
    // tile's, one of each on the die (Vivado's own placement of the probes).
    if matches!(bel, "DNA_PORT" | "EFUSE_USR") && !tile.sites.iter().any(|(_, k)| k == bel) {
        return Some((format!("{bel}_X0Y0"), bel.to_owned()));
    }
    if matches!(
        bel,
        "CAPTURE"
            | "DCIRESET"
            | "DNA_PORT"
            | "EFUSE_USR"
            | "FRAME_ECC"
            | "STARTUP"
            | "USR_ACCESS"
            | "XADC"
            | "IDELAYCTRL"
    ) {
        let mut of_kind = tile.sites.iter().filter(|(_, kind)| kind == bel);
        if let (Some((site, kind)), None) = (of_kind.next(), of_kind.next()) {
            return Some((site.clone(), kind.clone()));
        }
    }
    None
}

/// A buffer of a differential pair, as Vivado has it: the pad pin on the first
/// ball, the pin on the second, and the primitive. `None` for one this does
/// not know.
fn differential_of(primitive: &str) -> Option<(&'static str, &'static str, &str)> {
    Some(match primitive {
        "IBUF" => ("I", "IB", "IBUFDS"),
        "IBUF_IBUFDISABLE" => ("I", "IB", "IBUFDS_IBUFDISABLE"),
        "IBUF_INTERMDISABLE" => ("I", "IB", "IBUFDS_INTERMDISABLE"),
        "OBUF" => ("O", "OB", "OBUFDS"),
        "OBUFT" => ("O", "OB", "OBUFTDS"),
        "IOBUF" => ("IO", "IOB", "IOBUFDS"),
        "IOBUF_INTERMDISABLE" => ("IO", "IOB", "IOBUFDS_INTERMDISABLE"),
        other if other.starts_with("IOBUFDS") => ("IO", "IOB", other),
        other if other.starts_with("OBUFDS") || other.starts_with("OBUFTDS") => ("O", "OB", other),
        other if other.starts_with("IBUFDS") || other.starts_with("IBUFGDS") => ("I", "IB", other),
        _ => return None,
    })
}

/// One lookup table function as Vivado will see it: the pins it reads, with
/// the physical `A<n>` each one is, and its table over just those.
struct LutHalf {
    /// `(physical input, its connection)`, `I0` first.
    pins: Vec<(usize, SigBit)>,
    /// `2^pins` entries.
    table: Vec<bool>,
}

/// Reduces a truth table to the pins Vivado should see: drops each pin tied
/// to a constant that the table ignores. A pin tied to a constant it does
/// *not* ignore stays, tied, since that is what the netlist says.
fn reduce(table: &[bool], connections: &[SigBit]) -> LutHalf {
    let depends = |pin: usize| {
        (0..table.len()).any(|m| m & (1 << pin) == 0 && table[m] != table[m | (1 << pin)])
    };
    let mut pins: Vec<(usize, SigBit)> = connections
        .iter()
        .enumerate()
        .filter(|(pin, bit)| !matches!(bit, SigBit::Const(_)) || depends(*pin))
        .map(|(pin, bit)| (pin, *bit))
        .collect();
    // A table of no inputs is a constant. A `LUT1` needs one, so it keeps the
    // first pin, which it ignores, tied high: the mapper ties it low and the
    // router leaves it alone, so on the part it reads the one an input with
    // no pip on it reads, and Vivado leaves a pin tied high just so. Tied
    // low, Vivado routed a ground to it (`IMUX<n>.GFAN<k>` and
    // `GFAN<k>.GND_WIRE`) that Reticle's bitstream does not have, on every
    // probe with a constant in it (`gnd_probe`, `vcc_probe`, `ram16x1s_probe`).
    if pins.is_empty() {
        pins.push((0, SigBit::Const(Bit::One)));
    }
    let mut reduced = vec![false; 1 << pins.len()];
    for (m, entry) in reduced.iter_mut().enumerate() {
        let mut at = 0usize;
        for (i, (pin, _)) in pins.iter().enumerate() {
            if m & (1 << i) != 0 {
                at |= 1 << pin;
            }
        }
        *entry = table[at];
    }
    LutHalf {
        pins,
        table: reduced,
    }
}

/// `2^k` truth table bits as a sized Verilog literal.
fn init_literal(table: &[bool]) -> String {
    let mut hex = String::new();
    let digits = table.len().div_ceil(4);
    for digit in (0..digits).rev() {
        let mut value = 0u32;
        for bit in 0..4 {
            if table.get(digit * 4 + bit).copied().unwrap_or(false) {
                value |= 1 << bit;
            }
        }
        let _ = write!(hex, "{value:x}");
    }
    format!("{}'h{hex}", table.len())
}

/// A parameter's value in Verilog.
fn param_value(value: &AttrValue) -> String {
    match value {
        AttrValue::Const(c) => const_literal(c),
        AttrValue::String(s) => format!("\"{s}\""),
        AttrValue::Int(i) => i.to_string(),
    }
}

fn const_literal(c: &Const) -> String {
    let width = c.width();
    if width == 0 {
        return "1'b0".to_owned();
    }
    let bits: Vec<bool> = (0..width).map(|i| c.bit(i) == Bit::One).collect();
    if width == 1 {
        return format!("1'b{}", u8::from(bits[0]));
    }
    let mut hex = String::new();
    let digits = (width as usize).div_ceil(4);
    for digit in (0..digits).rev() {
        let mut value = 0u32;
        for bit in 0..4 {
            if bits.get(digit * 4 + bit).copied().unwrap_or(false) {
                value |= 1 << bit;
            }
        }
        let _ = write!(hex, "{value:x}");
    }
    format!("{width}'h{hex}")
}

/// The module's bits as names, and the writing of a connection.
struct Names<'a> {
    view: &'a BitView<'a>,
    /// A port bit's name, by slot, since a wire there is the port itself.
    ports: HashMap<usize, String>,
}

impl Names<'_> {
    fn slot(&self, slot: usize) -> String {
        if let Some(port) = self.ports.get(&slot) {
            return port.clone();
        }
        let (net, bit) = self.view.owner(slot);
        let name = safe(self.view.module().nets[net].name.as_str());
        format!("n{slot}_{name}_{bit}")
    }

    fn bit(&self, bit: SigBit) -> String {
        match bit {
            SigBit::Const(Bit::One) => "1'b1".to_owned(),
            SigBit::Const(_) => "1'b0".to_owned(),
            SigBit::Slot(slot) => self.slot(slot),
        }
    }

    /// A connection of any width, most significant bit first.
    fn bits(&self, bits: &[SigBit]) -> String {
        match bits {
            [one] => self.bit(*one),
            _ => {
                let items: Vec<String> = bits.iter().rev().map(|b| self.bit(*b)).collect();
                format!("{{{}}}", items.join(", "))
            }
        }
    }
}

/// The cell's connection on `port`, as bits; empty when it has none.
fn inputs_of(
    view: &BitView<'_>,
    cell: &Cell,
    port: &str,
) -> Result<Vec<SigBit>, VivadoExportError> {
    for (name, expr) in &cell.inputs {
        if name.as_str() == port {
            return view
                .expr_bits(*expr)
                .map(|bits| bits.into_iter().map(|b| view.canonical(b)).collect())
                .map_err(|e| VivadoExportError(e.to_string()));
        }
    }
    Ok(Vec::new())
}

fn output_of(
    view: &BitView<'_>,
    cell: &Cell,
    port: &str,
) -> Result<Vec<SigBit>, VivadoExportError> {
    for (name, net) in &cell.outputs {
        if name.as_str() == port {
            return view
                .net_bits(*net, cell.span)
                .map_err(|e| VivadoExportError(e.to_string()));
        }
    }
    Ok(Vec::new())
}

/// Accumulates the netlist's instances and the placement script's lines.
struct Writer<'a> {
    names: Names<'a>,
    verilog: String,
    placed: Vec<String>,
    locks: Vec<String>,
    cells: usize,
}

impl Writer<'_> {
    /// One lookup table function on one bel.
    fn lut(
        &mut self,
        name: &str,
        half: &LutHalf,
        output: SigBit,
        site: &str,
        bel: &str,
    ) -> Result<(), VivadoExportError> {
        let k = half.pins.len();
        if k > 6 {
            return fail(format!("`{name}` has {k} inputs"));
        }
        let mut pins = String::new();
        let mut lock = Vec::new();
        for (i, (physical, bit)) in half.pins.iter().enumerate() {
            let _ = write!(pins, ".I{i}({}), ", self.names.bit(*bit));
            lock.push(format!("I{i}:A{}", physical + 1));
        }
        let _ = writeln!(
            self.verilog,
            "  LUT{k} #(.INIT({})) {name} ({pins}.O({}));",
            init_literal(&half.table),
            self.names.bit(output)
        );
        self.placed.push(format!("  {name} {site}/{bel}"));
        self.locks.push(format!(
            "set_property LOCK_PINS {{{}}} [get_cells {name}]",
            lock.join(" ")
        ));
        self.cells += 1;
        Ok(())
    }

    /// One cell as it stands, on one bel.
    fn plain(
        &mut self,
        name: &str,
        cell: &Cell,
        view: &BitView<'_>,
        primitive: &str,
        extra: &[String],
    ) -> Result<(), VivadoExportError> {
        let structural = |e: crate::ir::emit::EmitError| VivadoExportError(e.to_string());
        let mut items = Vec::new();
        for (port, expr) in &cell.inputs {
            let bits: Vec<SigBit> = view
                .expr_bits(*expr)
                .map_err(structural)?
                .into_iter()
                .map(|b| view.canonical(b))
                .collect();
            items.push(format!(".{port}({})", self.names.bits(&bits)));
        }
        for (port, net) in &cell.outputs {
            let bits = view.net_bits(*net, cell.span).map_err(structural)?;
            items.push(format!(".{port}({})", self.names.bits(&bits)));
        }
        items.extend(extra.iter().cloned());
        let params: Vec<String> = cell
            .params
            .iter()
            .map(|(k, v)| format!(".{k}({})", param_value(v)))
            .collect();
        let head = if params.is_empty() {
            String::new()
        } else {
            format!(" #({})", params.join(", "))
        };
        let _ = writeln!(
            self.verilog,
            "  {primitive}{head} {name} ({});",
            items.join(", ")
        );
        Ok(())
    }
}

/// The cell's truth table of `2^bits` entries, read off its `INIT`.
fn truth_table(cell: &Cell, bits: usize) -> Vec<bool> {
    let init = cell.params.get("INIT").and_then(|v| match v {
        AttrValue::Const(c) => Some(c),
        _ => None,
    });
    (0..1u32 << bits)
        .map(|i| init.is_some_and(|c| i < c.width() && c.bit(i) == Bit::One))
        .collect()
}

/// A wire `sites.rs` invented inside a slice, which Vivado has no name for.
fn inside_a_site(wire: &str) -> bool {
    wire.contains("_MUX_") || wire.contains("_CARRY_")
}

/// Writes a route as a tree of `<tile>/<wire>` names: a node, then each pip
/// out of it, with every branch but the last in braces, which is the shape
/// Vivado's own `ROUTE` property has.
fn route_tree(
    route: &Route,
    graph: &RoutingGraph,
    tile_names: &HashMap<(u32, u32), &str>,
) -> String {
    let mut children: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for pip in &route.pips {
        let pip = graph.pip(*pip);
        children.entry(pip.from).or_default().push(pip.to);
    }
    let mut out = String::new();
    // An explicit stack, since a clock's tree is as deep as it is long.
    enum Step {
        Node(NodeId),
        Open,
        Close,
    }
    let mut stack = vec![Step::Node(route.source)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Open => out.push_str(" {"),
            Step::Close => out.push_str(" }"),
            Step::Node(node) => {
                let wire = graph.wire(node);
                if !inside_a_site(&wire.name)
                    && let Some(tile) = tile_names.get(&wire.tile)
                {
                    let _ = write!(out, " {tile}/{}", wire.name);
                }
                let next = children.get(&node).map_or(&[][..], Vec::as_slice);
                // Pushed in reverse: the last child goes on with no braces,
                // after every other, so it is pushed first.
                if let Some((last, rest)) = next.split_last() {
                    stack.push(Step::Node(*last));
                    for child in rest.iter().rev() {
                        stack.push(Step::Close);
                        stack.push(Step::Node(*child));
                        stack.push(Step::Open);
                    }
                }
            }
        }
    }
    out
}

/// Writes a routed design for Vivado; see the module's documentation.
///
/// `tiles` is the database's whole grid, whose coordinates are the graph's.
///
/// # Errors
///
/// When a cell's bel has no spelling here or its connections are not
/// structural: Vivado would be told something other than what was built, and
/// so nothing is written.
pub fn vivado_implementation(
    design: &Design,
    top: ModuleId,
    tiles: &[XrayTile],
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    routing: &Routing,
) -> Result<VivadoImplementation, VivadoExportError> {
    let module = &design.modules[top];
    let view = BitView::new(module).map_err(|e| VivadoExportError(e.to_string()))?;
    let at: HashMap<(u32, u32), &XrayTile> =
        tiles.iter().map(|t| ((t.grid_x, t.grid_y), t)).collect();
    let tile_names: HashMap<(u32, u32), &str> = tiles
        .iter()
        .map(|t| ((t.grid_x, t.grid_y), t.name.as_str()))
        .collect();

    // The ports, which name the bits they hold.
    let mut port_names: HashMap<usize, String> = HashMap::new();
    let mut header: Vec<String> = Vec::new();
    let mut port_lines = String::new();
    for port in &module.ports {
        let name = port.name.as_str();
        let bits = view
            .net_bits(port.net, port.span)
            .map_err(|e| VivadoExportError(e.to_string()))?;
        header.push(name.to_owned());
        let dir = match port.dir {
            crate::ir::PortDir::In => "input",
            crate::ir::PortDir::Out => "output",
            crate::ir::PortDir::InOut => "inout",
        };
        if bits.len() == 1 {
            let _ = writeln!(port_lines, "  {dir} {name};");
        } else {
            let _ = writeln!(port_lines, "  {dir} [{}:0] {name};", bits.len() - 1);
        }
        for (index, bit) in bits.iter().enumerate() {
            let SigBit::Slot(slot) = bit else {
                return fail(format!("port `{name}` bit {index} is a constant"));
            };
            let spelled = if bits.len() == 1 {
                name.to_owned()
            } else {
                format!("{name}[{index}]")
            };
            if port_names.insert(*slot, spelled).is_some() {
                return fail(format!("port `{name}` bit {index} is another port's wire"));
            }
        }
    }

    // The cell of each instance, and the name each gets. Cells nothing placed
    // cannot be written: the placer holds them all.
    let mut writer = Writer {
        names: Names {
            view: &view,
            ports: port_names,
        },
        verilog: String::new(),
        placed: Vec::new(),
        locks: Vec::new(),
        cells: 0,
    };
    for (index, instance) in netlist.instances.iter().enumerate() {
        let cell = &module.cells[instance.cell];
        if !matches!(cell.kind, CellKind::Blackbox(_)) {
            return fail(format!("`{}` is not a primitive", instance.name));
        }
        let Some(site) = placement.site_of(index) else {
            return fail(format!("`{}` was not placed", instance.name));
        };
        let site = &graph.sites[site];
        let Some(tile) = at.get(&site.tile) else {
            return fail(format!(
                "`{}` is on `{}`, in a tile the grid does not have",
                instance.name, site.name
            ));
        };
        let spell = |lower: bool| {
            site_and_bel(tile, &site.bel, &instance.primitive, lower).ok_or_else(|| {
                VivadoExportError(format!(
                    "no spelling for `{}` on bel `{}` of `{}`",
                    instance.primitive, site.bel, tile.name
                ))
            })
        };
        let name = format!("c{index}_{}", safe(&instance.name));
        match instance.primitive.as_str() {
            "LUT6" | "LUT5" | "LUT4" | "LUT3" | "LUT2" | "LUT1" => {
                let k = instance.primitive[3..].parse::<usize>().unwrap_or(6);
                let mut pins = Vec::with_capacity(k);
                for i in 0..k {
                    let bits = inputs_of(&view, cell, &format!("I{i}"))?;
                    pins.push(bits.first().copied().unwrap_or(SigBit::Const(Bit::Zero)));
                }
                let table = truth_table(cell, k);
                let half = reduce(&table, &pins);
                let output = output_of(&view, cell, "O")?;
                let Some(output) = output.first() else {
                    return fail(format!("`{}` has no output", instance.name));
                };
                let (s, b) = spell(false)?;
                writer.lut(&name, &half, *output, &s, &b)?;
            }
            "LUT6_2" => {
                let mut pins = Vec::with_capacity(6);
                for i in 0..6 {
                    let bits = inputs_of(&view, cell, &format!("I{i}"))?;
                    pins.push(bits.first().copied().unwrap_or(SigBit::Const(Bit::Zero)));
                }
                let table = truth_table(cell, 6);
                let lower = &table[..32];
                // `O6` is the upper half with `I5` high, the one way a pair
                // is built here; with a signal on `I5` it is the whole table.
                let (upper, upper_pins): (&[bool], &[SigBit]) = match pins[5] {
                    SigBit::Const(Bit::One) => (&table[32..], &pins[..5]),
                    SigBit::Const(_) => (lower, &pins[..5]),
                    SigBit::Slot(_) => (&table[..], &pins[..]),
                };
                if let Some(o6) = output_of(&view, cell, "O6")?.first() {
                    let (s, b) = spell(false)?;
                    let half = reduce(upper, upper_pins);
                    writer.lut(&format!("{name}__O6"), &half, *o6, &s, &b)?;
                }
                if let Some(o5) = output_of(&view, cell, "O5")?.first() {
                    let (s, b) = spell(true)?;
                    let half = reduce(lower, &pins[..5]);
                    writer.lut(&format!("{name}__O5"), &half, *o5, &s, &b)?;
                }
            }
            primitive if primitive.starts_with("RAM") && !primitive.starts_with("RAMB") => {
                // A distributed RAM is a macro of Vivado's, placed by its
                // leaves, which `testdata/fpga/vivado/lutram/vivado.txt` names
                // as Vivado names them. A `_1` is the cell with its clock
                // inverted, as Vivado has it.
                let (s, _) = spell(false)?;
                let lower = site.bel.ends_with("_LO");
                let (base, inverted) = match primitive.strip_suffix("_1") {
                    Some(base) => (base, true),
                    None => (primitive, false),
                };
                let leaves: Vec<(String, String)> = match base {
                    "RAM64X1D" | "RAM32X1D" => {
                        let n = if lower { 5 } else { 6 };
                        vec![
                            ("SP".to_owned(), format!("D{n}LUT")),
                            ("DP".to_owned(), format!("C{n}LUT")),
                        ]
                    }
                    "RAM64X1S" | "RAM32X1S" => vec![("SP".to_owned(), "D6LUT".to_owned())],
                    "RAM64X2S" | "RAM32X2S" => {
                        let one = base.replace("X2S", "X1S");
                        vec![
                            (format!("{one}0/SP"), "D6LUT".to_owned()),
                            (format!("{one}1/SP"), "C6LUT".to_owned()),
                        ]
                    }
                    "RAM64M" => ['A', 'B', 'C', 'D']
                        .iter()
                        .map(|l| (format!("RAM{l}"), format!("{l}6LUT")))
                        .collect(),
                    "RAM32M" => ['A', 'B', 'C', 'D']
                        .iter()
                        .flat_map(|l| {
                            [
                                (format!("RAM{l}"), format!("{l}5LUT")),
                                (format!("RAM{l}_D1"), format!("{l}6LUT")),
                            ]
                        })
                        .collect(),
                    // With their wide multiplexers, as Vivado names the leaves
                    // of `memarr/sp_as_128x1` and `sp_as_256x1` placed: only
                    // counted, since Vivado places the macro by its site.
                    "RAM128X1S" => [("LOW", "D6LUT"), ("HIGH", "C6LUT"), ("F7", "F7BMUX")]
                        .map(|(l, b)| (l.to_owned(), b.to_owned()))
                        .to_vec(),
                    "RAM256X1S" => [
                        ("RAMS64E_A", "A6LUT"),
                        ("RAMS64E_B", "B6LUT"),
                        ("RAMS64E_C", "C6LUT"),
                        ("RAMS64E_D", "D6LUT"),
                        ("F7.A", "F7AMUX"),
                        ("F7.B", "F7BMUX"),
                        ("F8", "F8MUX"),
                    ]
                    .map(|(l, b)| (l.to_owned(), b.to_owned()))
                    .to_vec(),
                    // The others' wide multiplexers are leaves too, and how
                    // Vivado names those has not been asked.
                    _ => {
                        return fail(format!(
                            "`{}` is a `{primitive}`, which is not written for Vivado yet",
                            instance.name
                        ));
                    }
                };
                let mut written = cell.clone();
                if inverted {
                    written
                        .params
                        .set("IS_WCLK_INVERTED", AttrValue::Const(Const::from_u64(1, 1)));
                }
                writer.plain(&name, &written, &view, base, &[])?;
                // Vivado places a `RAM32M`, a `RAM64M`, a `RAM64X2S`, a
                // `RAM128X1S` or a `RAM256X1S` by its site alone, and expands
                // it by its own shape, which is this fabric's: placed leaf by
                // leaf, it says `Cannot expand macro instance` (12-1409) on
                // any site, alone in the design (the last two measured on
                // `memarr/sp_as_256x2` and `sp_rf_256x24`).
                if matches!(
                    base,
                    "RAM32M" | "RAM64M" | "RAM64X2S" | "RAM128X1S" | "RAM256X1S"
                ) {
                    writer.placed.push(format!("  {name} {s}"));
                    writer.cells += leaves.len();
                } else {
                    for (leaf, bel) in leaves {
                        writer.placed.push(format!("  {name}/{leaf} {s}/{bel}"));
                        writer.cells += 1;
                    }
                }
            }
            primitive => {
                // Vivado knows one `BUFR`; see `regional`.
                let mut primitive = match primitive {
                    super::regional::REGIONAL_BUFR => "BUFR",
                    super::regional::REGIONAL_BUFMRCE => "BUFMRCE",
                    other => other,
                };
                // A multi-region buffer this flow built on a global one
                // (`--regional-clocks auto`) is that global buffer to Vivado,
                // with the same pins: `BUFMRCE` on `BUFGCTRL` is refused
                // (12-1409).
                if site.bel.starts_with("BUFGCTRL_") {
                    primitive = match primitive {
                        "BUFMRCE" => "BUFGCE",
                        "BUFMR" => "BUFG",
                        other => other,
                    };
                }
                // The second ball of a differential pair, which this flow
                // configures from the buffer on the first and which has no
                // cell of its own: Vivado's buffer is the differential one,
                // with the pair's `_N` port on its second pad pin.
                let mut extra: Vec<String> = Vec::new();
                if cell
                    .attrs
                    .get("io_pair")
                    .and_then(AttrValue::as_int)
                    .is_some_and(|v| v != 0)
                {
                    let Some((_, second, differential)) = differential_of(primitive) else {
                        return fail(format!(
                            "`{}` is a `{primitive}` of a differential pair, which is not \
                             written for Vivado yet",
                            instance.name
                        ));
                    };
                    // The pair's other port, which mapping names on the buffer
                    // (`io_pair_port`), and the bit of it the buffer's own name
                    // says (`<port>$io<bit>`, `primitives::io_buffer`).
                    let bit = instance
                        .name
                        .rsplit_once("$io")
                        .and_then(|(_, bit)| bit.parse::<usize>().ok())
                        .unwrap_or(0);
                    let partner = cell
                        .attrs
                        .get("io_pair_port")
                        .and_then(AttrValue::as_str)
                        .and_then(|n| {
                            module.ports.iter().find(|p| {
                                p.name.as_str() == n || module.nets[p.net].name.as_str() == n
                            })
                        })
                        .and_then(|port| view.net_bits(port.net, port.span).ok())
                        .and_then(|bits| bits.get(bit).copied());
                    let Some(partner) = partner else {
                        return fail(format!(
                            "`{}` is half of a differential pair whose other port is not found",
                            instance.name
                        ));
                    };
                    extra.push(format!(".{second}({})", writer.names.bit(partner)));
                    primitive = differential;
                }
                let (s, b) = site_and_bel(tile, &site.bel, primitive, false).ok_or_else(|| {
                    VivadoExportError(format!(
                        "no spelling for `{primitive}` on bel `{}` of `{}`",
                        site.bel, tile.name
                    ))
                })?;
                // A serialiser's `DATA_RATE`, which Vivado's design rules
                // want stated (`ADEF-2`), at the library's default.
                let mut written = cell.clone();
                if primitive == "ISERDESE2" && written.params.get("DATA_RATE").is_none() {
                    written
                        .params
                        .set("DATA_RATE", AttrValue::String("DDR".to_owned()));
                }
                // An `ISERDESE2`'s `CLKB` (and `OCLKB`) on the very net of its
                // `CLK` (`OCLK`) is that clock's inverse to the site, which
                // inverts it itself and has no bit for it: the netlist says
                // so, or Vivado's rule that the two be opposite refuses it
                // (`PDRC-26`).
                if primitive == "ISERDESE2" {
                    for (clock, inverse) in [("CLK", "CLKB"), ("OCLK", "OCLKB")] {
                        let a = inputs_of(&view, cell, clock)?;
                        let b = inputs_of(&view, cell, inverse)?;
                        let key = format!("IS_{inverse}_INVERTED");
                        if !a.is_empty() && a == b && written.params.get(&key).is_none() {
                            written
                                .params
                                .set(key.as_str(), AttrValue::Const(Const::from_u64(1, 1)));
                        }
                    }
                }
                let cell = &written;
                writer.plain(&name, cell, &view, primitive, &extra)?;
                // A macro is placed by its site and expanded by Vivado.
                if b.is_empty() {
                    writer.placed.push(format!("  {name} {s}"));
                } else {
                    writer.placed.push(format!("  {name} {s}/{b}"));
                }
                writer.cells += 1;
                // A pull the pad's buffer carries, which a `PULLUP`,
                // `PULLDOWN` or `KEEPER` of the design's, or a constraint,
                // put there: Vivado's own cell for it, on the pad's `PULL`
                // bel, where its own flow puts one.
                if site.bel.starts_with("IOB_") {
                    let flag = |key: &str| {
                        cell.attrs
                            .get(key)
                            .and_then(AttrValue::as_int)
                            .is_some_and(|v| v != 0)
                    };
                    let pull = if flag("keeper") {
                        Some("KEEPER")
                    } else if flag("pullup") {
                        Some("PULLUP")
                    } else if flag("pulldown") {
                        Some("PULLDOWN")
                    } else {
                        None
                    };
                    let pad = if primitive.starts_with("IBUF") {
                        inputs_of(&view, cell, "I")?
                    } else if primitive.starts_with("IOBUF") {
                        let mut bits = inputs_of(&view, cell, "IO")?;
                        if bits.is_empty() {
                            bits = output_of(&view, cell, "IO")?;
                        }
                        bits
                    } else {
                        output_of(&view, cell, "O")?
                    };
                    if let (Some(pull), Some(bit)) = (pull, pad.first()) {
                        let _ = writeln!(
                            writer.verilog,
                            "  {pull} {name}__pull (.O({}));",
                            writer.names.bit(*bit)
                        );
                        writer.placed.push(format!("  {name}__pull {s}/PULL"));
                        writer.cells += 1;
                    }
                }
            }
        }
    }

    // Every wire a cell or a port names, declared.
    let mut wires: Vec<String> = Vec::new();
    for slot in 0..view.slots() {
        if let SigBit::Slot(root) = view.canonical(SigBit::Slot(slot))
            && root == slot
            && !writer.names.ports.contains_key(&slot)
        {
            wires.push(writer.names.slot(slot));
        }
    }

    let mut verilog = String::new();
    let _ = writeln!(
        verilog,
        "// Reticle's implementation of `{}`, for Vivado's link_design.",
        module.name
    );
    let _ = writeln!(verilog, "module {} ({});", module.name, header.join(", "));
    verilog.push_str(&port_lines);
    for wire in &wires {
        let _ = writeln!(verilog, "  wire {wire};");
    }
    verilog.push_str(&writer.verilog);
    verilog.push_str("endmodule\n");

    let mut script = String::new();
    script.push_str("# Reticle's placement, for a design already linked.\n");
    script.push_str("place_cell {\n");
    for line in &writer.placed {
        script.push_str(line);
        script.push('\n');
    }
    script.push_str("}\n");
    for line in &writer.locks {
        script.push_str(line);
        script.push('\n');
    }
    script.push_str(
        "set placed [get_cells -hierarchical -filter {IS_PRIMITIVE && STATUS != UNPLACED}]\n\
         set_property IS_BEL_FIXED 1 $placed\n\
         set_property IS_LOC_FIXED 1 $placed\n",
    );

    // Each routed signal, on the net its driver's output is.
    let mut routes = String::new();
    routes.push_str("# Reticle's routes, as <tile>/<wire> trees.\n");
    let mut nets = 0usize;
    for route in routing.routes() {
        let signal = &netlist.signals[route.signal];
        let Some(driver) = signal.driver else {
            continue;
        };
        let pin = &netlist.pins[driver];
        let cell = &module.cells[netlist.instances[pin.instance].cell];
        let bits = output_of(&view, cell, &pin.port)?;
        let Some(bit) = bits.get(pin.bit as usize) else {
            return fail(format!(
                "`{}` drives `{}` on a bit its port does not have",
                netlist.instances[pin.instance].name, signal.name
            ));
        };
        let net = writer.names.bit(*bit);
        let tree = route_tree(route, graph, &tile_names);
        let _ = writeln!(routes, "reticle_route {{{net}}} {{{tree} }}");
        nets += 1;
    }

    Ok(VivadoImplementation {
        verilog,
        placement: script,
        routes,
        cells: writer.cells,
        nets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_input_the_table_ignores_is_left_out_with_the_table_cut_to_match() {
        // I0 AND I2 of three, with I1 tied low and ignored: a table of two
        // inputs, on the physical pins they arrived on.
        let table: Vec<bool> = (0..8).map(|m| m & 1 != 0 && m & 4 != 0).collect();
        let half = reduce(
            &table,
            &[SigBit::Slot(7), SigBit::Const(Bit::Zero), SigBit::Slot(9)],
        );
        assert_eq!(half.pins, vec![(0, SigBit::Slot(7)), (2, SigBit::Slot(9))]);
        assert_eq!(half.table, vec![false, false, false, true]);
    }

    /// Would catch a pair's or a mapper's table that does read a tied input
    /// losing that input; dropping it would change what the cell computes.
    #[test]
    fn a_tied_input_the_table_reads_stays() {
        let table: Vec<bool> = (0..4).map(|m| m & 2 != 0).collect();
        let half = reduce(&table, &[SigBit::Slot(1), SigBit::Const(Bit::One)]);
        assert_eq!(
            half.pins,
            vec![(0, SigBit::Slot(1)), (1, SigBit::Const(Bit::One))]
        );
    }

    /// Would catch the constant's one pin written tied low again, which
    /// Vivado routes a ground to where Reticle's bitstream has no pip.
    #[test]
    fn a_table_of_no_inputs_is_still_a_cell_with_its_pin_tied_high() {
        let half = reduce(
            &[true; 4],
            &[SigBit::Const(Bit::Zero), SigBit::Const(Bit::Zero)],
        );
        assert_eq!(half.table, vec![true, true]);
        assert_eq!(half.pins, vec![(0, SigBit::Const(Bit::One))]);
    }

    #[test]
    fn init_is_written_most_significant_first_at_its_width() {
        let table: Vec<bool> = (0..4).map(|m| m == 3).collect();
        assert_eq!(init_literal(&table), "4'h8");
        let table: Vec<bool> = (0..2).map(|m| m == 0).collect();
        assert_eq!(init_literal(&table), "2'h1");
        let table: Vec<bool> = (0..32).map(|m| m == 31).collect();
        assert_eq!(init_literal(&table), "32'h80000000");
    }

    fn tile(sites: &[(&str, &str)]) -> XrayTile {
        XrayTile {
            name: "T".to_owned(),
            tile_type: "CLBLM_L".to_owned(),
            grid_x: 0,
            grid_y: 0,
            sites: sites
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect(),
            bits: Vec::new(),
            bits_alias: None,
        }
    }

    /// A slice's bels are spelled as `get_bels` spells them; the file's other
    /// checks of these spellings are Vivado's refusal when one is wrong.
    #[test]
    fn a_blocks_bel_is_the_site_of_its_rank_up_the_tile() {
        let dsp = tile(&[
            ("DSP48_X2Y39", "DSP48E1"),
            ("DSP48_X2Y38", "DSP48E1"),
            ("TIEOFF_X50Y95", "TIEOFF"),
        ]);
        assert_eq!(
            site_and_bel(&dsp, "DSP_0", "DSP48E1", false),
            Some(("DSP48_X2Y38".to_owned(), "DSP48E1".to_owned()))
        );
        assert_eq!(
            site_and_bel(&dsp, "DSP_1", "DSP48E1", false),
            Some(("DSP48_X2Y39".to_owned(), "DSP48E1".to_owned()))
        );
    }

    #[test]
    fn a_slice_bel_is_spelled_as_vivado_spells_it() {
        let t = tile(&[("SLICE_X10Y3", "SLICEM"), ("SLICE_X11Y3", "SLICEL")]);
        let spell = |bel, lower| site_and_bel(&t, bel, "LUT6", lower).unwrap();
        assert_eq!(
            spell("SLICEM_X0_ALUT", false),
            ("SLICE_X10Y3".to_owned(), "A6LUT".to_owned())
        );
        assert_eq!(
            spell("SLICEL_X1_DLUT", true),
            ("SLICE_X11Y3".to_owned(), "D5LUT".to_owned())
        );
        assert_eq!(
            spell("SLICEL_X1_B5FF", false),
            ("SLICE_X11Y3".to_owned(), "B5FF".to_owned())
        );
        assert_eq!(
            spell("SLICEL_X1_CARRY4", false),
            ("SLICE_X11Y3".to_owned(), "CARRY4".to_owned())
        );
        assert_eq!(
            spell("SLICEL_X1_CLATCH", false),
            ("SLICE_X11Y3".to_owned(), "CFF".to_owned())
        );
        assert_eq!(
            spell("SLICEM_X0_BSRL32", false),
            ("SLICE_X10Y3".to_owned(), "B6LUT".to_owned())
        );
        assert_eq!(
            spell("SLICEM_X0_DSRL16", false),
            ("SLICE_X10Y3".to_owned(), "D6LUT".to_owned())
        );
    }

    /// The spellings added when every primitive probe went through Vivado
    /// (2026-11-06), each as Vivado's own placement of the probe named it
    /// (`get_property BEL` of each leaf after `place_design`). Would catch one
    /// lost, a macro given a bel (which `place_cell` refuses: Vivado expands
    /// it), or a global buffer that is not a `BUFG` put on `BUFG`'s bel, which
    /// Vivado refused as `BUFG_BUFG` for a `BUFGCTRL`.
    #[test]
    fn every_probes_primitive_is_spelled_as_vivado_placed_it() {
        let io = tile(&[("IOB_X1Y144", "IOB33M"), ("IOB_X1Y143", "IOB33S")]);
        let on_pad = |primitive| site_and_bel(&io, "IOB_Y0", primitive, false).unwrap().1;
        for primitive in [
            "IBUF_IBUFDISABLE",
            "IBUF_INTERMDISABLE",
            "IBUFDS",
            "IBUFGDS",
        ] {
            assert_eq!(on_pad(primitive), "INBUF_EN", "{primitive}");
        }
        for primitive in ["OBUFDS", "OBUFTDS"] {
            assert_eq!(on_pad(primitive), "OUTBUF", "{primitive}");
        }
        for primitive in [
            "IOBUF",
            "IOBUFDS_DIFF_OUT",
            "IBUFDS_IBUFDISABLE",
            "IBUFDS_DIFF_OUT",
        ] {
            assert_eq!(on_pad(primitive), "", "{primitive} is a macro");
        }
        let config = tile(&[
            ("BSCAN_X0Y0", "BSCAN"),
            ("BSCAN_X0Y1", "BSCAN"),
            ("ICAP_X0Y1", "ICAP"),
            ("STARTUP_X0Y0", "STARTUP"),
            ("FRAME_ECC_X0Y0", "FRAME_ECC"),
        ]);
        let spell = |bel, primitive| site_and_bel(&config, bel, primitive, false);
        assert_eq!(
            spell("BSCAN_X0Y1", "BSCANE2"),
            Some(("BSCAN_X0Y1".to_owned(), "BSCAN".to_owned()))
        );
        assert_eq!(
            spell("ICAP_X0Y1", "ICAPE2"),
            Some(("ICAP_X0Y1".to_owned(), "ICAP".to_owned()))
        );
        assert_eq!(
            spell("STARTUP", "STARTUPE2"),
            Some(("STARTUP_X0Y0".to_owned(), "STARTUP".to_owned()))
        );
        assert_eq!(
            spell("FRAME_ECC", "FRAME_ECCE2"),
            Some(("FRAME_ECC_X0Y0".to_owned(), "FRAME_ECC".to_owned()))
        );
        let clocks = tile(&[("BUFGCTRL_X0Y0", "BUFGCTRL"), ("BUFGCTRL_X0Y1", "BUFGCTRL")]);
        for (primitive, bel) in [
            ("BUFG", "BUFG"),
            ("BUFGCE", "BUFGCTRL"),
            ("BUFGMUX", "BUFGCTRL"),
        ] {
            assert_eq!(
                site_and_bel(&clocks, "BUFGCTRL_X0Y1", primitive, false),
                Some(("BUFGCTRL_X0Y1".to_owned(), bel.to_owned())),
                "{primitive}"
            );
        }
    }

    /// The pair's buffer for each single one this flow builds on the first
    /// ball. Would catch an `OBUFT` of a pair written as an `OBUFDS`, which
    /// has no `T`, or a pin of the second ball misnamed.
    #[test]
    fn a_pairs_buffer_is_the_differential_one() {
        assert_eq!(differential_of("IBUF"), Some(("I", "IB", "IBUFDS")));
        assert_eq!(differential_of("OBUFT"), Some(("O", "OB", "OBUFTDS")));
        assert_eq!(differential_of("IOBUF"), Some(("IO", "IOB", "IOBUFDS")));
        assert_eq!(
            differential_of("IBUFDS_DIFF_OUT"),
            Some(("I", "IB", "IBUFDS_DIFF_OUT"))
        );
        assert_eq!(differential_of("BUFG"), None);
    }
}
