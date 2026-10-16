// Stand-in for the Vivado XADC Wizard core `xadc_wiz_0` of NewAE's
// ChipWhisperer-Husky gateware: the `XADC` primitive with the wizard's
// initial registers, its DRP passed straight through.
//
// The registers are the `C_CONFIGURATION_R*`, `C_SEQUENCE_R*` and
// `C_ALARM_LIMIT_R*` values of `xadc_wiz_0.xci` (chipwhisperer-husky-fpga
// 2dfe1bc), in hexadecimal, and each was checked against the XADC tile of
// NewAE's own image as decoded here — with one exception: `INIT_5C`, which
// the `.xci` gives as `C_ALARM_LIMIT_R12 = 0x9999` and Vivado wrote as
// `0x5111`. This is Vivado's value. `src/fpga/xray/xadc.rs` has the
// register layout and `tests/fpga_xray_xadc.rs` the comparison.
//
// Which alarm is which bit of `ALM` is UG480's table for the 7 series
// (`ALM[0]` temperature, `[1]` VCCINT, `[2]` VCCAUX, `[3]` VCCBRAM,
// `[7]` any of them): quoted, not measured.
module xadc_wiz_0 (
    input  wire [15:0] di_in,
    input  wire [ 6:0] daddr_in,
    input  wire        den_in,
    input  wire        dwe_in,
    output wire        drdy_out,
    output wire [15:0] do_out,
    input  wire        dclk_in,
    input  wire        reset_in,
    input  wire        vp_in,
    input  wire        vn_in,
    output wire        user_temp_alarm_out,
    output wire        vccint_alarm_out,
    output wire        vccaux_alarm_out,
    output wire        vbram_alarm_out,
    output wire        ot_out,
    output wire [ 4:0] channel_out,
    output wire        eoc_out,
    output wire        alarm_out,
    output wire        eos_out,
    output wire        busy_out
);
  wire [7:0] alm;
  XADC #(
      .INIT_40(16'h0000), .INIT_41(16'h20a0), .INIT_42(16'h0400),
      .INIT_48(16'h4700), .INIT_49(16'h0000), .INIT_4A(16'h4700), .INIT_4B(16'h0000),
      .INIT_4C(16'h0000), .INIT_4D(16'h0000), .INIT_4E(16'h0000), .INIT_4F(16'h0000),
      .INIT_50(16'hb363), .INIT_51(16'h5999), .INIT_52(16'ha147), .INIT_53(16'hb5e3),
      .INIT_54(16'ha93a), .INIT_55(16'h5111), .INIT_56(16'h91eb), .INIT_57(16'ha93a),
      .INIT_58(16'h5999), .INIT_5C(16'h5111)
  ) u_xadc (
      .DCLK(dclk_in), .DEN(den_in), .DWE(dwe_in), .DADDR(daddr_in), .DI(di_in),
      .DO(do_out), .DRDY(drdy_out), .RESET(reset_in), .CONVST(1'b0), .CONVSTCLK(1'b0),
      .VP(vp_in), .VN(vn_in), .VAUXP(16'h0000), .VAUXN(16'h0000),
      .ALM(alm), .OT(ot_out)
  );
  assign user_temp_alarm_out = alm[0];
  assign vccint_alarm_out = alm[1];
  assign vccaux_alarm_out = alm[2];
  assign vbram_alarm_out = alm[3];
  assign alarm_out = alm[7];
  // Nothing in the Husky reads these.
  assign channel_out = 5'd0;
  assign eoc_out = 1'b0;
  assign eos_out = 1'b0;
  assign busy_out = 1'b0;
endmodule
