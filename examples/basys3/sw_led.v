// Two slide switches through one LUT to one LED, on a Digilent Basys 3.
//
// This is the smallest design that exercises the whole 7-series chain:
// three IO buffers, one LUT, no clock, no global buffer, no memory. A
// human verifies it by flipping a switch and watching LED 0, which is
// why it is the milestone design rather than anything with a display.
//
// A plain `assign led = sw0` would not serve: synthesis turns that into
// a wire, no LUT survives, and nothing carries a truth table into the
// bitstream. The exclusive-or needs a lookup table and is still a
// one-glance test.
//
//   sw0 (V17)  sw1 (V16)  led (U16)
//      0          0          off
//      1          0          on
//      0          1          on
//      1          1          off
//
// NOTHING PRODUCED FROM THIS HAS BEEN LOADED INTO A PART. See
// `docs/fpga-xray.md` for what the flow does and does not establish.
module sw_led (
    input  wire sw0,
    input  wire sw1,
    output wire led
);
    assign led = sw0 ^ sw1;
endmodule
