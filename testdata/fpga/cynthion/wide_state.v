// A register with a bit nothing ever sets, which is the shape of the defect
// that kept a USB device from enumerating for eight rounds of looking
// elsewhere.
//
// `state` is three bits wide and takes four values, so `state[2]` is a
// flip-flop whose data input is the constant zero. On a Lattice ECP5 that
// flip-flop's data comes from the fabric's `M` wire, and an unrouted slice
// input on this family reads as a **one** — Lattice's own packer ties unused
// lookup-table inputs high and this flow does the same — so the bit comes up
// **set**, and `state` reads 5 where the design can only produce 0 to 3.
//
// `ip/usb_device_fs/rtl/usb_ctrl_ep.v` had exactly this in its `stage`
// register. Every `case (stage)` label missed, every IN token a host sent was
// answered from the `default` arm with a NAK, and the host's transfer died of a
// five second timeout with the device's receive path, its transmit path and its
// SETUP acknowledgement all working. `docs/fpga-trellis.md` has the measurement
// that finally read the 5 off the part.
//
// So `TrellisFabric::configure_registers` refuses this rather than writing a
// device that does something else, and this design is what it is asked of.
// **It is not meant to build.**

module wide_state (
    input  wire clk,
    output wire led0_n
);
    reg [2:0] state = 3'd0;
    always @(posedge clk) begin
        // Four values, and never a fifth: the top bit is the constant zero.
        state <= (state == 3'd3) ? 3'd0 : (state + 3'd1) & 3'd3;
    end
    assign led0_n = ~state[0];
endmodule
