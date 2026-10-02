// The thirteen balls of a Cynthion's TARGET USB transceiver, driven, read
// back and turned around — all of them on the **left** edge of the die,
// which is the edge this design exists to exercise.
//
// ===================================================================
// DO NOT LOAD THIS ONTO A BOARD
// ===================================================================
//
// **There is nothing on a Cynthion r1.4 that can show whether this design
// works.** The six FPGA LEDs are on the top edge of the die and the USER
// button is on the right; this design touches neither, on purpose, because
// mixing edges is how a wrong left-edge rule gets hidden behind a right-edge
// one that works. So a person looking at the board would see exactly what
// they see with any other bitstream loaded: nothing moving, `DONE` high.
//
// That is why this design is verified **off the part**, by
// `tests/fpga_trellis.rs::the_target_ulpi_design_routes_and_configures_what_its_header_promises`,
// which places it, routes it, and checks that every set bit of the resulting
// bitstream decodes back through the database to the arcs the router chose.
// `docs/fpga-trellis.md` says what a board would have added and what the
// cheapest experiment to get it would be.
//
// **And before anybody loads it anyway**, two things on the list in
// `CLAUDE.md` have to be looked at again, because this design drives a port
// that goes somewhere:
//
// 1. **Is the PHY really held in reset by a low on R4?** This design drives
//    `target_rst_n` to a constant zero, and the only evidence for what that
//    does is Great Scott Gadgets' platform file, which spells the pin
//    `rst="R4", rst_invert=True` — Amaranth's `PinsN`, so their logical
//    "assert reset" is a low on the ball. A Microchip USB3343's `RESETB` is
//    active low, which agrees. **Neither of those is the board's schematic**,
//    and this file has not read it.
// 2. **Is a driver on `target_data` while `target_dir` is low safe?** That
//    is the ULPI bus rule — the transceiver owns the bus while `dir` is
//    high — and this design obeys it. But it obeys it with the PHY in
//    reset, where `dir` is not driven at all, which is why the constraints
//    ask for a pull-up on `dir` and `nxt`: an undriven `dir` has to read as
//    "the PHY owns the bus" and not as "go ahead and drive".
//
// ===================================================================
// WHAT IT DOES
// ===================================================================
//
//   * **`target_clk` (T4)** carries the 60 MHz oscillator divided by two, so
//     an output pad on this edge has something changing on it. It is not a
//     ULPI clock: a real one is the oscillator itself and would have to come
//     out of the clock network, which `clock_blink.v` is the design for.
//   * **`target_data[7:0]` (R2 R1 P2 P1 N3 N1 M2 M1)** is driven with the
//     counter's low byte while `target_dir` reads low, and **released** —
//     high impedance — while it reads high. Eight bidirectional pads on one
//     edge, each with a tristate that a *route* drives rather than a tie.
//   * **`target_stp` (T3)** is the way back in. It is the exclusive-or of
//     all eight data pads *as read through their own input buffers* and of
//     `target_nxt`, so the one output bit depends on nine input paths of
//     this edge. A left-edge input buffer that read nothing would leave it
//     stuck; one that read the wrong ball would leave it moving wrongly.
//   * **`target_rst_n` (R4)**, **`target_c_vbus_en` (K5)**,
//     **`control_vbus_en` (L1)** and **`aux_vbus_en` (L2)** are constant
//     zeros, which is four output pads whose data comes from a `CIB` tie
//     rather than from a route. The three switches are active **high** in
//     the platform file — plain `Pins`, not `PinsN` — so zero is **off**,
//     and nothing here enables VBUS anywhere, which is the rule in
//     `CLAUDE.md`.
//
// So the design uses, on one edge: two input pads, eight bidirectional pads
// with routed tristates, two routed output pads, four tied output pads, and
// one bank rail (bank 6). The only ball not on that edge is the clock.
//
// ===================================================================
// WHAT WOULD BE WRONG, AND WHAT IT WOULD LOOK LIKE
// ===================================================================
//
// Nothing here can be seen on the board, so this list is about what the
// off-part checks would show:
//
//   * **the pad tile one row out** — every pad is configured in the tile of
//     a *different ball of the same edge*, the bitstream still decodes, and
//     the comparison against `analyzer.bit` in
//     `what_lattices_own_packer_writes_for_a_left_edge_pad` is the only
//     thing that notices. That is the failure this whole exercise is about;
//   * **the `CIB` one column out** — the four tied outputs tie a wire of
//     another tile, and `the_bitstream_decodes_back_to_the_arcs_the_router_chose`
//     reports bits that belong to no arc of this design;
//   * **the tristate's sense inverted** — `target_data` would drive while
//     the transceiver drives, which on a real part is a bus fight. That is
//     why the design is built with `1'bz` and not with an enable the
//     technology mapper has to guess the polarity of; `oen=` in
//     `src/fpga/devices/ecp5.dev` says which way round it is, and two of the
//     four device files had it the wrong way round once.
//
// Pins: testdata/fpga/cynthion/target_ulpi_loopback.rcf.

module target_ulpi_loopback (
    input  wire       clk,

    // The transceiver's own pins, all thirteen, in the platform file's
    // directions.
    output wire       target_clk,     // T4, the FPGA drives this
    inout  wire [7:0] target_data,    // R2 R1 P2 P1 N3 N1 M2 M1
    input  wire       target_dir,     // R3, the transceiver drives this
    input  wire       target_nxt,     // T2, likewise
    output wire       target_stp,     // T3
    output wire       target_rst_n,   // R4, low holds the PHY in reset

    // The three power switches, all off.
    output wire       target_c_vbus_en,  // K5
    output wire       control_vbus_en,   // L1
    output wire       aux_vbus_en        // L2
);

    reg [25:0] count = 26'd0;

    // `toggle[i]` is the carry into bit i, so `count ^ toggle` is
    // `count + 1`. Spelled as a reduction rather than as a chain because
    // `CCU2C` has no port map in `src/fpga/devices/ecp5.dev`;
    // `clock_blink.v` has the full argument and `tests/fpga_xray.rs` proves
    // the spelling equivalent to `count + 1`.
    wire [25:0] toggle;
    assign toggle[0] = 1'b1;

    genvar i;
    generate
        for (i = 1; i < 26; i = i + 1) begin : carry
            assign toggle[i] = &count[i-1:0];
        end
    endgenerate

    always @(posedge clk) begin
        count <= count ^ toggle;
    end

    // 30 MHz on an output pad of this edge.
    assign target_clk = count[0];

    // THE TURNAROUND. The ULPI rule is that the transceiver owns the bus
    // while `dir` is high, so the drivers are enabled by its inverse and
    // the eight pads are released the rest of the time.
    assign target_data = ~target_dir ? count[7:0] : 8'bz;

    // THE WAY BACK IN. Nine input paths of this edge reduced to one output
    // bit: the eight data pads read through their own input buffers, and
    // `nxt`.
    assign target_stp = (^target_data) ^ target_nxt;

    // Held in reset. See the header's first question.
    assign target_rst_n = 1'b0;

    // Off. Active high in the platform file, so zero is off, and nothing in
    // this repository may enable a VBUS switch.
    assign target_c_vbus_en = 1'b0;
    assign control_vbus_en  = 1'b0;
    assign aux_vbus_en      = 1'b0;

endmodule
