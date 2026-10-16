// The XADC as NewAE's ChipWhisperer-Husky configures it through Vivado's
// `xadc_wiz`: continuous sequence mode over the on-chip sensors with
// averaging, and the temperature, VCCINT, VCCAUX and VBRAM alarms armed.
// The initial registers are the wizard's, from `xadc_wiz_0.xci`, with
// `INIT_5C` as Vivado wrote it (see `src/fpga/xray/xadc.rs`). Its DRP is
// driven from the fabric and its alarms reach the LEDs.
//
// `tests/fpga_xray_xadc.rs` builds it with `reticle fpga --bitstream` and
// compares the XADC's tile with NewAE's image. Nothing here has run on a
// part.
module xadc_demo (
    input  wire clk_usb,
    output wire LED_ADC,
    output wire LED_GLITCH,
    output wire LED_ARMED
);
  wire clk;
  BUFG u_bufg (.I(clk_usb), .O(clk));
  reg [15:0] count = 0;
  always @(posedge clk) count <= count + 1;

  wire [15:0] do_out;
  wire drdy, ot;
  wire [7:0] alm;
  XADC #(
      .INIT_40(16'h0000), .INIT_41(16'h20a0), .INIT_42(16'h0400),
      .INIT_48(16'h4700), .INIT_49(16'h0000), .INIT_4A(16'h4700), .INIT_4B(16'h0000),
      .INIT_4C(16'h0000), .INIT_4D(16'h0000), .INIT_4E(16'h0000), .INIT_4F(16'h0000),
      .INIT_50(16'hb363), .INIT_51(16'h5999), .INIT_52(16'ha147), .INIT_53(16'hb5e3),
      .INIT_54(16'ha93a), .INIT_55(16'h5111), .INIT_56(16'h91eb), .INIT_57(16'ha93a),
      .INIT_58(16'h5999), .INIT_5C(16'h5111)
  ) u_xadc (
      .DCLK(clk), .DEN(count[0]), .DWE(1'b0), .DADDR(count[7:1]), .DI(16'h0000),
      .DO(do_out), .DRDY(drdy), .RESET(count[15]), .CONVST(1'b0), .CONVSTCLK(1'b0),
      .VP(1'b0), .VN(1'b0), .VAUXP(16'h0000), .VAUXN(16'h0000),
      .ALM(alm), .OT(ot)
  );
  assign LED_ADC = ^do_out ^ drdy;
  assign LED_GLITCH = ot ^ alm[0];
  assign LED_ARMED = |alm[3:1];
endmodule
