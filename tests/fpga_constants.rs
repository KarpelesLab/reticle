//! **No primitive input is left holding a constant**, for every bel role
//! rather than for one.
//!
//! This is the general form of `no_carry_pin_is_left_holding_a_constant`
//! in `src/fpga/primitives.rs`, which asserts it of a `CCU2C`'s operands
//! alone. That test exists because a constant on a carry operand cost a
//! board to find; this one exists because the same fault then turned up
//! twice more, on pins nobody had thought to list:
//!
//! - a **block RAM's address**. An ECP5 `DP16KD` in 18-bit mode wants
//!   `AD[3:0] = 0011` and two of those bits are a constant **zero**, which
//!   an unrouted ECP5 input reads as a **one**, so every 18-bit block this
//!   flow built addressed its contents through `1111`.
//! - an **output buffer's data**. On the 7 series `assign led = 1'b1` left
//!   the pad's input unrouted and the pad came out **low** on a Basys 3,
//!   with no diagnostic.
//!
//! The two remedies are not the same, and this test is keyed to that: the
//! two pins above get a **driver**, while a 7-series `RAMB18E1`'s unused
//! low address bits — which the block does not read — get the idle tie
//! policy in `src/fpga/xray/bram.rs` instead, because what went wrong there
//! was a build *failing* over a contended ground fan rather than a value
//! being read. So `bram`/`addr` appears in the exemption table for four
//! families and not for the ECP5, and `io`/`dout` is excused only by the
//! one device file that declares `absorbs dout`.
//!
//! # What this would and would not catch
//!
//! It catches the fault class *structurally*, which is the only way it can
//! be caught: **simulation cannot see this at all.** A netlist has no
//! unrouted wires, so a constant pin evaluates to the constant in every
//! simulation, in `every_block_maps_to_the_logic_it_was_mapped_from`, in
//! the exhaustive model checks in `src/fpga/primitives.rs` and in the SAT
//! proof in `tests/fpga_carry.rs` — all of which passed while the carry
//! chain computed `cnt - 1` on real silicon. Every bit of the bitstream
//! decoded too. The assertion therefore has to be about the *shape* of
//! the netlist and not about what it computes.
//!
//! It would **not** have caught either of the two faults above by itself,
//! and that is worth being exact about: a constant pin is only wrong
//! because of what the fabric does with an unrouted wire, and this test
//! does not know that. What it does is refuse to let a *new* pin join the
//! class silently. The exemption table below is the list of pins a family
//! designs to take a constant, each with the mechanism that absorbs it,
//! and a constant anywhere else fails here by cell, port and bit.
//!
//! It also would not catch a pin the flow leaves **unconnected** rather
//! than constant. That is a different defect with a different remedy —
//! `XrayFabric::configure_block_rams`'s tie pass and
//! `TrellisFabric::configure_registers`'s refusal — and
//! `tests/fpga_xray_bram.rs` is where it is checked.

#![cfg(all(feature = "fpga", feature = "verilog", feature = "synth"))]

use std::collections::{BTreeMap, BTreeSet};

use reticle::diag::Diagnostics;
use reticle::fpga::device::Device;
use reticle::fpga::{Constraints, FpgaOptions, synthesize_for, target};
use reticle::ir::emit::{BitView, SigBit};
use reticle::ir::{Bit, CellKind};
use reticle::source::SourceMap;

/// Every device this tree ships, so the question is asked of each family's
/// own port names and not of one family's.
const DEVICES: [&str; 6] = [
    "ecp5-12f-CABGA256",
    "xc7a35t-cpg236",
    "ice40-hx1k-tq144",
    "gw2a-18-pg256",
    "generic",
    "generic-k6",
];

/// Designs chosen so that one of them puts a constant on each pin the
/// flow has ever left one on: a counter (a carry operand), a register
/// wider than its values (a flip-flop's data), a memory (a block RAM's or
/// a distributed RAM's address and data), two constant output pads, and a
/// bidirectional pad (a tristate).
///
/// `odd_counter` is five bits rather than sixteen on purpose: a 7-series
/// `CARRY4` covers four bits, so a width that is not a multiple of four
/// leaves lanes nothing uses and their `S` and `DI` are constants. Those
/// are the two pins `src/fpga/xray/carry.rs` absorbs into the lane's own
/// truth table, and nothing exercised them before.
const DESIGNS: [(&str, &str); 6] = [
    (
        "counter",
        "module top(input clk, output [15:0] q);\n\
         reg [15:0] cnt = 0;\n\
         always @(posedge clk) cnt <= cnt + 1;\n\
         assign q = cnt;\n\
         endmodule\n",
    ),
    (
        "odd_counter",
        "module top(input clk, output [4:0] q);\n\
         reg [4:0] cnt = 0;\n\
         always @(posedge clk) cnt <= cnt + 1;\n\
         assign q = cnt;\n\
         endmodule\n",
    ),
    (
        "wide_state",
        "module top(input clk, input go, output [2:0] stage);\n\
         reg [2:0] s = 0;\n\
         always @(posedge clk) s <= go ? 3'd2 : 3'd1;\n\
         assign stage = s;\n\
         endmodule\n",
    ),
    (
        "memory",
        "module top(input clk, input we, input [9:0] addr, input [7:0] din,\n\
         output reg [7:0] dout);\n\
         reg [7:0] mem [0:1023];\n\
         always @(posedge clk) begin\n\
         if (we) mem[addr] <= din;\n\
         dout <= mem[addr];\n\
         end\n\
         endmodule\n",
    ),
    (
        "constant_pads",
        "module top(input a, output y, output high, output low);\n\
         assign y = a;\n\
         assign high = 1'b1;\n\
         assign low = 1'b0;\n\
         endmodule\n",
    ),
    (
        "tristate",
        "module top(input oe, input d, inout io, output q);\n\
         assign io = oe ? d : 1'bz;\n\
         assign q = io;\n\
         endmodule\n",
    ),
];

/// One pin bit that is still a constant after the whole flow.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Held {
    /// The device's role keyword for the primitive (`lut`, `ff`, `io`).
    kind: String,
    /// The abstract pin role the device knows the port by (`d`, `ci`).
    role: String,
    primitive: String,
    cell: String,
    port: String,
    bit: u32,
    /// The constant, as the character a bit renders as, because
    /// `ir::Bit` is not `Ord` and this struct is sorted for a stable
    /// message.
    value: String,
    /// The device's own `.dev` file says this family's backend applies a
    /// constant on this pin itself — a `bel … absorbs <role>` clause. The
    /// exemption is then per device rather than global, which is the whole
    /// point of putting it in the file: an ECP5 pad's data wire can be
    /// tied in its `CIB` tile and a 7-series `OBUF`'s cannot.
    declared: bool,
}

/// The pins a family **designs** to take a constant, as
/// `(role keyword, pin role, why)`.
///
/// This table is written out here rather than read from
/// `techcells::absorbs_constant`, on purpose: a test that asked the pass
/// which pins the pass skips would pass by construction. Every row is a
/// mechanism that has been read in a database, a vendor bitstream or this
/// flow's own code, and a constant on anything not in it fails.
const ABSORBED: [(&str, &str, &str); 16] = [
    (
        "lut",
        "i",
        "the truth table. `map_lut` widens a narrow table so the function ignores the tied \
         inputs, and Lattice's own packer does it from the other end: `SLICE<l>.<X><n>MUX = 1` on \
         every unused input with the value folded into `INIT`, in all 4262 carry halves of this \
         board's own bitstreams",
    ),
    (
        "ff",
        "clk",
        "a clock is not a data pin, and a lookup table on one would be a clock arriving on \
         general routing, which both fabric backends refuse by name",
    ),
    (
        "ff",
        "en",
        "a mux bit the backend writes: `SLICE<l>.CEMUX = 1` on an ECP5, and on the 7 series a \
         cleared `CEUSEDMUX` ties `CE` to one, which is what a flip-flop with no enable wants",
    ),
    (
        "ff",
        "rst",
        "likewise `LSR<c>.LSRMUX` and a cleared `SRUSEDMUX`, which ties `SR` to zero",
    ),
    (
        "carry",
        "ci",
        "dedicated metal from the cell below; nothing outside the chain may drive it, and a \
         lookup table on one would describe a connection the fabric has not got",
    ),
    (
        "carry",
        "cyinit",
        "the pin a family designs as its way in: `WideCarry` is \"the chain goes on `ci` and the \
         constant on `init`\"",
    ),
    (
        "carry",
        "p",
        "the lane's own truth table. A wide carry's propagate input is a 7-series `CARRY4`'s `S`, \
         and `src/fpga/xray/carry.rs` gives a constant one a lookup table of that lane rather \
         than a driver from elsewhere. The narrow shapes' operands — `a`, `b`, `i0`, `i1` — are \
         not in this table: those are the pins that cost a board",
    ),
    (
        "carry",
        "di",
        "the same lane's truth table, in its **lower half**: a `CARRY4`'s `DI` is reached from \
         the slice's own `O5` and not from general routing, so a driver elsewhere would cost a \
         lookup table and a route to a pin the fabric wires short",
    ),
    (
        "io",
        "oe",
        "the direction recipe. Which buffer a port gets, and which `BASE_TYPE` an ECP5 pad gets, \
         is decided from this pin, and the ECP5 then ties the wire itself (`CIB.JB0MUX`) — a tie \
         that is on every Cynthion bitstream this flow has put in a part",
    ),
    ("io", "oen", "the same pin under the other family's name"),
    (
        "gb",
        "en",
        "a constant this flow puts there on purpose: `primitives` ties an ECP5 `DCCA`'s `CE` high \
         because nothing gates a clock here",
    ),
    (
        "other",
        "ci",
        "a Gowin `ALU`'s carry in is the same dedicated metal as a `carry`'s, under a role \
         keyword the device file spells `other`",
    ),
    (
        "other",
        "oen",
        "a Gowin `TBUF`/`IOBUF` tristate is the same pin as an `io`'s `oen`",
    ),
    (
        "bram",
        "addr",
        "a bit below the width mode's word address, which the block does not read: a `RAMB18E1` \
         at 18 bits uses `ADDRARDADDR[13:4]` and an `SB_RAM40_4K` at 8 bits `ADDR[8:0]`. That is \
         their libraries speaking and not an instrument here — which is why the 7-series tie pass \
         gives such a bit `TiePolicy::Idle`, so a contended ground fan cannot fail a build over \
         one. The family that *does* read them says so, with `ecp5.dev`'s `pad 4'b0011`, and \
         those four bits get a driver: that is this round's defect",
    ),
    (
        "bram",
        "din",
        "the mapper's own padding. A write-data line outside the mode's width is a line the block \
         does not store, and that is what a width mode means. **A constant the design puts \
         inside the width is not covered and is a known gap** — `mem[a] <= {4'b0, x}` on an \
         eight-bit memory would read ones back on an ECP5. Closing it means routing eight to ten \
         more sinks into every block, which did not fit the synthetic iCE40's RAM tile in \
         `tests/fpga_pnr.rs`; the 7-series tie pass covers it after routing and the ECP5's does \
         not. Stated rather than hidden",
    ),
    (
        "bram",
        "en",
        "a constant **one**, which is what every design here puts on it, and an unrouted input \
         reads one on both families with a bitstream backend — so it is right either way. A \
         constant **zero** would mean a port that is off, and that is the case the tie passes \
         were written for; whether a *netlist* zero on an ECP5 `CEB` reaches them is **not** \
         checked here, and nothing in this tree exercises it",
    ),
];

/// `(primitive, port)` to `(role keyword, pin role)`, from the device's own
/// declarations: its `bel` lines' port maps and its `bram` blocks'.
fn roles_of(device: &Device) -> BTreeMap<(String, String), (String, String, bool)> {
    let mut out = BTreeMap::new();
    for bel in &device.bels {
        for (role, names) in &bel.ports {
            for name in names.split(',').filter(|n| !n.is_empty()) {
                out.insert(
                    (bel.name.clone(), name.to_owned()),
                    (
                        bel.role.keyword().to_owned(),
                        role.clone(),
                        bel.absorbs_constant(role),
                    ),
                );
            }
        }
    }
    for bram in &device.block_rams {
        for port in &bram.port_map {
            for (role, name) in &port.signals {
                out.insert(
                    (bram.name.clone(), name.clone()),
                    ("bram".to_owned(), role.clone(), false),
                );
            }
        }
    }
    for dsp in &device.dsps {
        for (role, name) in &dsp.ports {
            out.insert(
                (dsp.name.clone(), name.clone()),
                ("dsp".to_owned(), role.clone(), false),
            );
        }
        for (name, _) in &dsp.ties {
            out.insert(
                (dsp.name.clone(), name.clone()),
                ("dsp".to_owned(), "tie".to_owned(), false),
            );
        }
    }
    out
}

/// Runs the whole flow on `verilog` for `device` and returns every
/// primitive input bit that is still a constant.
fn held_constants(name: &str, verilog: &str, device: &str) -> (Vec<Held>, usize) {
    let device = target(device).unwrap_or_else(|| panic!("`{device}` is a built-in device"));
    let mut map = SourceMap::new();
    let source = map.add(format!("{name}.v"), verilog).unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        source,
        reticle::verilog::Dialect::Verilog2005,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    let mut design = reticle::verilog::elaborate_file(
        &ast,
        &reticle::verilog::ElabOptions::default(),
        &mut diags,
    )
    .expect("the design elaborates");
    let top = design.top.expect("a top module");
    synthesize_for(
        &mut design,
        top,
        device,
        &Constraints::new(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the flow");
    assert!(
        !diags.has_errors(),
        "{name} on {}: {}",
        device.name,
        diags.render(&map)
    );

    let roles = roles_of(device);
    let module = design.module(top);
    let view = BitView::new(module).expect("a structural netlist");
    let mut held = Vec::new();
    let mut drivers = BTreeSet::new();
    for (_, cell) in module.cells.iter() {
        let CellKind::Blackbox(primitive) = &cell.kind else {
            continue;
        };
        // The constant drivers themselves, so the test can check there is
        // one per constant and not one per pin.
        if cell.name.as_str().starts_with("const0$") || cell.name.as_str().starts_with("const1$") {
            drivers.insert(cell.name.as_str().to_owned());
        }
        for (port, value) in &cell.inputs {
            let Ok(bits) = view.expr_bits(*value) else {
                continue;
            };
            for (index, bit) in bits.iter().enumerate() {
                let SigBit::Const(value @ (Bit::Zero | Bit::One)) = view.canonical(*bit) else {
                    continue;
                };
                let key = (primitive.as_str().to_owned(), port.as_str().to_owned());
                let (kind, role, declared) = roles.get(&key).cloned().unwrap_or_else(|| {
                    panic!(
                        "`{}` declares no port map entry for `{}`.{}, so nothing can say whether \
                         a constant belongs on it",
                        device.name,
                        primitive.as_str(),
                        port.as_str()
                    )
                });
                held.push(Held {
                    kind,
                    role,
                    primitive: primitive.as_str().to_owned(),
                    cell: cell.name.as_str().to_owned(),
                    port: port.as_str().to_owned(),
                    bit: u32::try_from(index).unwrap_or(0),
                    value: value.to_string(),
                    declared,
                });
            }
        }
    }
    held.sort();
    (held, drivers.len())
}

/// **No input pin of any primitive is left holding a constant**, except
/// the pins [`ABSORBED`] names and for the reasons it gives.
///
/// What this would catch: a role, or a pin of a role, that starts holding
/// a constant — because a new primitive was mapped, or because a pass
/// stopped driving one it used to. It names the cell, the port and the bit.
///
/// What it would not catch: that a constant on a *listed* pin is still
/// absorbed. Those eleven rows are readings of a database, of a vendor
/// bitstream or of this flow's own code, and if one of them stops being
/// true nothing here will say so.
#[test]
fn no_primitive_input_is_left_holding_a_constant() {
    let mut unexpected: Vec<String> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut declared: BTreeSet<(&str, String, String)> = BTreeSet::new();
    for device in DEVICES {
        for (name, verilog) in DESIGNS {
            let (held, drivers) = held_constants(name, verilog, device);
            for pin in &held {
                if pin.declared {
                    declared.insert((device, pin.kind.clone(), pin.role.clone()));
                    continue;
                }
                seen.insert((pin.kind.clone(), pin.role.clone()));
                let absorbed = pin.declared
                    || ABSORBED
                        .iter()
                        .any(|(kind, role, _)| *kind == pin.kind && *role == pin.role);
                if !absorbed {
                    unexpected.push(format!(
                        "{device} / {name}: `{}`.{}[{}] of `{}` is the constant {} — a {} pin in \
                         role `{}`, which nothing says can absorb one",
                        pin.cell, pin.port, pin.bit, pin.primitive, pin.value, pin.kind, pin.role
                    ));
                }
            }
            // One lookup table per constant, shared by every pin that
            // wants that value — which is what nextpnr's `pack_constants`
            // does and what Lattice's own bitstreams for a Cynthion
            // contain. Never one per pin.
            assert!(
                drivers <= 2,
                "{device} / {name}: {drivers} constant drivers, and there are only two constants"
            );
        }
    }
    assert!(
        unexpected.is_empty(),
        "{} primitive pin(s) left holding a constant:\n{}",
        unexpected.len(),
        unexpected.join("\n")
    );
    // Which absorbed roles these designs actually reach, pinned exactly.
    // A role appearing is caught by the loop above; a role *disappearing*
    // is caught here, and is worth catching — it means a pass stopped
    // putting a constant somewhere, which is either the point of a change
    // or a surprise.
    let seen: Vec<(&str, &str)> = seen
        .iter()
        .map(|(kind, role)| (kind.as_str(), role.as_str()))
        .collect();
    assert_eq!(
        seen, EXERCISED,
        "the absorbed roles these designs reach have changed"
    );
    // And the per-device exemptions, which are the device files' own
    // `absorbs` clauses rather than this table's rows. There is one, and
    // naming it here is what keeps it from growing unnoticed.
    let declared: Vec<(&str, &str, &str)> = declared
        .iter()
        .map(|(device, kind, role)| (*device, kind.as_str(), role.as_str()))
        .collect();
    assert_eq!(
        declared,
        [("ecp5-12f-CABGA256", "io", "dout")],
        "the `absorbs` clauses these designs reach have changed. A pin excused by a device file \
         rather than by the table above is a family saying it has a mechanism another family has \
         not got, and there should be very few"
    );
}

/// The rows of [`ABSORBED`] these designs actually put a constant on, in
/// sorted order.
///
/// It is shorter than [`ABSORBED`], and the gap is itself a measurement:
///
/// - **`ff`/`clk`** — nothing ever puts a constant on a clock. The row is
///   there because the pass must not build a lookup table if something
///   ever does, not because anything does.
/// - **`io`/`oe` and `io`/`oen`** — an ECP5 `TRELLIS_IO` driving an
///   output leaves `T` **unconnected** rather than constant, and a
///   7-series `OBUF` has no tristate pin at all, so even the `tristate`
///   design here reaches the pin with a *signal*. The row covers the
///   families that would tie it.
/// - **`other`/`ci` and `other`/`oen`** — Gowin's `ALU` and `TBUF` are
///   declared in `gowin.dev` and nothing instantiates either: that family
///   declares no `carry` role, so no chain is inferred for it, and a
///   `TBUF` would need a port this set of designs does not have.
///
/// `bram`/`addr` is here for the four families whose files state no `pad`
/// value, and **not** for the ECP5: there those bits have a driver, which
/// is what `a_block_rams_address_and_a_pads_data_have_drivers` asserts.
/// `io`/`dout` is in neither list — the ECP5 excuses it in its own file,
/// which the per-device check at the end of the test pins, and every other
/// family drives it.
const EXERCISED: [(&str, &str); 11] = [
    ("bram", "addr"),
    ("bram", "din"),
    ("bram", "en"),
    ("carry", "ci"),
    ("carry", "cyinit"),
    ("carry", "di"),
    ("carry", "p"),
    ("ff", "en"),
    ("ff", "rst"),
    ("gb", "en"),
    ("lut", "i"),
];

/// The two pins this round added, asked directly rather than through the
/// general rule, so a regression says which of them came back.
///
/// **A block RAM address bit the family states a value for.** An ECP5
/// `DP16KD` in 18-bit mode addresses its words through `AD[13:4]` and
/// reads `AD[3:0]`, which must be `0011`; `ecp5.dev` says so with `pad
/// 4'b0011`; two of those four bits are a constant zero and an unrouted
/// input on that family is a **one**, so every 18-bit block RAM this flow
/// built addressed its contents through `1111`. Every bit of that address
/// has a driver now. A family whose file states no `pad` value is not
/// asserted here: those bits are ones its library says the block does not
/// read, and `ABSORBED` carries that reading with its reason.
///
/// **An output buffer's data pin, on a family that cannot tie it.**
/// `assign led = 1'b1` left a 7-series `OBUF`'s input unrouted and the pad
/// came out **low** on a Basys 3. An ECP5 pad is the exception and says so
/// in its own file (`bel TRELLIS_IO io absorbs dout`): its `CIB` tile can
/// tie the wire, `configure_io` writes that, and the tie is in every
/// bitstream this flow has put in a Cynthion — including the one whose
/// `R4` holds a constant zero and which a USB host enumerates.
///
/// What this would not catch: that the ECP5's tie is still correct. That
/// rests on the part, and on `the_target_ulpi_design_...` in
/// `tests/fpga_trellis.rs` asserting the bits.
#[test]
fn a_block_rams_address_and_a_pads_data_have_drivers() {
    let mut states_a_pad_value = 0usize;
    let mut can_tie_a_pad = 0usize;
    for device in DEVICES {
        let shapes = target(device).expect("a built-in device");
        let stated = shapes
            .block_rams
            .iter()
            .any(|bram| bram.mode_layouts.iter().any(|l| l.addr_pad.is_some()));
        let (held, _) = held_constants("memory", DESIGNS[3].1, device);
        let addresses: Vec<&Held> = held
            .iter()
            .filter(|pin| pin.kind == "bram" && pin.role == "addr")
            .collect();
        if stated {
            states_a_pad_value += 1;
            assert!(
                addresses.is_empty(),
                "{device}: a block RAM address bit is still a constant although this family's \
                 own `.dev` file states what those bits must read: {addresses:?}"
            );
        }

        let (held, _) = held_constants("constant_pads", DESIGNS[4].1, device);
        let pads: Vec<&Held> = held
            .iter()
            .filter(|pin| pin.role == "dout" || pin.role == "dout1")
            .collect();
        if pads.iter().all(|pin| pin.declared) {
            can_tie_a_pad += usize::from(!pads.is_empty());
            continue;
        }
        assert!(
            pads.is_empty(),
            "{device}: an output buffer's data pin is still a constant and nothing in this \
             family's file says the backend ties it: {pads:?}. The pad came out low on a Basys 3, \
             and `ecppack` routes a constant lookup table into all 318 output pads of its three \
             bitstreams for a Cynthion rather than tying one of them"
        );
    }
    assert_eq!(
        states_a_pad_value, 1,
        "one shipped family states a value for the bits below the word address, and it is the \
         ECP5. A second one appearing means this test covers something new"
    );
    assert_eq!(
        can_tie_a_pad, 1,
        "one shipped family declares `absorbs dout`, and it is the ECP5"
    );
}
