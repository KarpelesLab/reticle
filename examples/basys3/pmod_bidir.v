// One bidirectional pad on a Digilent Basys 3: a Pmod pin that drives
// when told to, lets go when not, and is read back through its own input
// buffer onto an LED.
//
// This is the design that settles the one thing about a 7-series
// tristate nothing on a desk could: WHICH WAY ROUND ITS ENABLE IS. The
// bit that decides it, `OLOGIC_Y<n>.ZINV_T1`, is quoted from Project
// X-Ray's fuzzer and from nextpnr-xilinx, and no Vivado bitstream this
// project has holds a tristate pin to measure it against. See
// `docs/fpga-xray.md`, "A tristate, and a pad that reads itself".
//
//   sw0 (V17)  slide switch SW0: 1 = drive the pin, 0 = release it
//   sw1 (V16)  slide switch SW1: the value driven
//   jc1 (K17)  Pmod JC pin 1, bidirectional, with the weak pull-up on
//   led0 (U16) LED LD0: what jc1 READS, through its own input buffer
//   led1 (E19) LED LD1: sw0, so a person can see "driving" at a glance
//
// WHAT A PERSON SHOULD SEE, with NOTHING plugged into header JC:
//
//   SW0   SW1   jc1 is      LD0   LD1
//   down  down  released    on    off    (the pull-up)
//   down  up    released    on    off    (SW1 has no effect)
//   up    down  driven 0    OFF   on
//   up    up    driven 1    on    on
//
// So LD0 is dark in exactly one of the four positions — SW0 up, SW1
// down — and from there, flipping SW0 down must light LD0 again at once:
// that is the pin let go and pulled up.
//
// What each way of being wrong looks like instead:
//
//   the enable inverted   LD0 follows SW1 while SW0 is DOWN, and stays
//                         lit whatever SW1 does while SW0 is up; the
//                         one dark position is SW0 down, SW1 down
//   no pull-up            LD0 with SW0 down is whatever the floating pin
//                         last held — typically it stays dark after
//                         SW0 up/SW1 down is left, instead of lighting
//   never driven          LD0 lit in all four positions
//   input path dead       LD0 never changes at all
//
// Why this pin, and why it cannot fight anything: K17 goes to the JC
// header and to nothing else Digilent's master constraints file
// (`Basys-3-Master.xdc`, "Pmod Header JC", `JC[0]`, "Sch name = JC1")
// names. With nothing plugged into JC the FPGA is the only driver the net
// has, whichever way the enable turns out to be: an inverted enable
// changes WHEN the pad drives, never whether something else drives too.
// Do not plug anything into JC for this, and no jumper wire is needed.
//
// WHAT HAS BEEN TRIED ON A PART: nothing. It builds, routes completely,
// and every bit of its bitstream decodes back to a feature the router or
// the placer asked for (`tests/fpga_xray_tristate.rs`).
module pmod_bidir (
    input  wire sw0,
    input  wire sw1,
    inout  wire jc1,
    output wire led0,
    output wire led1
);
    assign jc1 = sw0 ? sw1 : 1'bz;
    assign led0 = jc1;
    assign led1 = sw0;
endmodule
