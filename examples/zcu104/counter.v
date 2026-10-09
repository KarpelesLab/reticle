// A 32-bit counter on the ZCU104's fabric, clocked by the processor's
// PL_CLK0 and read by Linux on EMIO GPIO inputs 0..31.
//
// On 2026-10-09 this design, built by `reticle fpga --bitstream`, ran on a
// ZCU104 and counted at 100.000 MHz. Read it from Linux in one go — the
// 32 lines are one GPIO bank — at 0xFF0A006C (the GPIO block's DATA_3_RO
// register, EMIO 0..31). docs/fpga-uray.md has the whole procedure.
module counter (
    input  wire        clk,
    output reg  [31:0] count
);
  always @(posedge clk) count <= count + 32'd1;
endmodule
