// Stand-in for the 7-series `USR_ACCESSE2` primitive, which NewAE's
// ChipWhisperer-Husky gateware reads as its build time. On the part,
// `DATA` is the bitstream's `AXSS` register, which Vivado fills with a
// timestamp when told to; Reticle neither places this site nor writes
// that register, so the word is zero — what the Husky's own source gives
// it under Icarus (`reg_openadc.v`, the `__ICARUS__` branch). Software
// that prints the build time will print zero.
module USR_ACCESSE2 (
    output wire        CFGCLK,
    output wire [31:0] DATA,
    output wire        DATAVALID
);
  assign CFGCLK = 1'b0;
  assign DATA = 32'd0;
  assign DATAVALID = 1'b0;
endmodule
