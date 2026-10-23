// Behavioural simulation models of the Xilinx 7-series primitives NewAE's
// ChipWhisperer-Husky gateware instantiates, so that NewAE's own
// whole-design testbench (`fpga/sim/cwhusky_tb.v`) runs in Reticle's
// simulator. NewAE's makefile gives iverilog Vivado's `unisims` library for
// these; that library is not something this repository can carry, so these
// are written from the primitives' descriptions in UG953 (Libraries Guide),
// UG472 (Clocking Resources) and XAPP888 (MMCM and PLL Dynamic
// Reconfiguration). Each behaviour below is a reading of those, not a
// measurement, and each says what it leaves out.
//
// The clock managers are the substantial part. MMCME2_ADV and PLLE2_ADV
// measure the selected input's period, assert LOCKED after a few equal
// periods, and generate every output from (input period * DIVCLK_DIVIDE /
// CLKFBOUT_MULT) * divide with its duty cycle and phase, all outputs
// starting together on the input edge they lock on. Their DRP registers
// start from the parameters in XAPP888's encodings, and a write to an
// output's or the feedback's divide registers takes effect at the next
// lock, as NewAE's reconfiguration holds RST around the writes. Left out:
// the feedback path (CLKFBIN is not watched; the outputs are as if the
// feedback were the usual CLKFBOUT loop), fractional divides beyond what a
// real-valued period expresses, CLKOUT4_CASCADE, spread spectrum, jitter
// and the lock time's dependence on the frequencies. A fine phase-shift
// step moves the outputs that use fine phase by 1/56 of the VCO period at
// the next lock rather than at once.

`timescale 1ps / 1ps

module BUFG (output wire O, input wire I);
  assign O = I;
endmodule

module IBUFG #(parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT")
    (output wire O, input wire I);
  assign O = I;
endmodule

// `S` low selects `I0`. The real buffer switches glitch-free, waiting for
// both clocks to be low; this switches at once.
module BUFGMUX #(parameter CLK_SEL_TYPE = "SYNC")
    (output wire O, input wire I0, input wire I1, input wire S);
  assign O = S ? I1 : I0;
endmodule

// A regional buffer with an optional divider. `BUFR_DIVIDE` of 1 (or
// "BYPASS") passes the clock; 2 to 8 divide it, the output's first rising
// edge on the input's first, while `CE` is high and `CLR` low.
module BUFR #(parameter BUFR_DIVIDE = "BYPASS", parameter SIM_DEVICE = "7SERIES")
    (output reg O, input wire I, input wire CE, input wire CLR);
  localparam integer N = (BUFR_DIVIDE == "BYPASS" || BUFR_DIVIDE == 1 || BUFR_DIVIDE == "1") ? 1
                       : (BUFR_DIVIDE == 2 || BUFR_DIVIDE == "2") ? 2
                       : (BUFR_DIVIDE == 3 || BUFR_DIVIDE == "3") ? 3
                       : (BUFR_DIVIDE == 4 || BUFR_DIVIDE == "4") ? 4
                       : (BUFR_DIVIDE == 5 || BUFR_DIVIDE == "5") ? 5
                       : (BUFR_DIVIDE == 6 || BUFR_DIVIDE == "6") ? 6
                       : (BUFR_DIVIDE == 7 || BUFR_DIVIDE == "7") ? 7 : 8;
  integer count = 0;
  initial O = 1'b0;
  always @(I or CLR) begin
    if (CLR) begin
      O = 1'b0;
      count = 0;
    end else if (N == 1) begin
      O = I & CE;
    end else if (CE) begin
      // One output transition every N input half periods.
      if (count == 0) O = ~O;
      count = (count + 1) % N;
    end
  end
endmodule

module BUFMRCE #(parameter CE_INVERTED = 1'b0, parameter CE_TYPE = "SYNC",
                 parameter INIT_OUT = 0)
    (output wire O, input wire I, input wire CE);
  wire enable = CE ^ CE_INVERTED;
  assign O = enable ? I : INIT_OUT[0];
endmodule

// A weak pull to one on the net `O` is joined to. Vivado's model calls `O`
// an output; it is an `inout` here, since a pull is a weak driver on a net
// that other things drive too, and Reticle counts an `inout` as a bus.
module PULLUP (inout wire O);
  assign (pull1, highz0) O = 1'b1;
endmodule

// A differential input: `I` high and `IB` low read one, the reverse zero,
// anything else unknown.
module IBUFDS #(parameter DIFF_TERM = "FALSE", parameter IBUF_LOW_PWR = "TRUE",
                parameter IOSTANDARD = "DEFAULT", parameter DQS_BIAS = "FALSE")
    (output wire O, input wire I, input wire IB);
  assign O = (I === 1'b1 && IB === 1'b0) ? 1'b1 : (I === 1'b0 && IB === 1'b1) ? 1'b0 : 1'bx;
endmodule

module OBUFDS #(parameter IOSTANDARD = "DEFAULT", parameter SLEW = "SLOW")
    (output wire O, output wire OB, input wire I);
  assign O = I;
  assign OB = ~I;
endmodule

// Output double data rate. OPPOSITE_EDGE: D1 at the rising edge, D2 at the
// falling. SAME_EDGE: both taken at the rising edge, D2 shown at the falling.
module ODDR #(parameter DDR_CLK_EDGE = "OPPOSITE_EDGE", parameter INIT = 1'b0,
              parameter SRTYPE = "SYNC")
    (output reg Q, input wire C, input wire CE, input wire D1, input wire D2,
     input wire R, input wire S);
  reg d2_held;
  initial Q = INIT;
  // One block for both edges, so that `Q` has one writer.
  always @(posedge C or negedge C or posedge R or posedge S) begin
    if (SRTYPE == "ASYNC" && R) Q <= 1'b0;
    else if (SRTYPE == "ASYNC" && S) Q <= 1'b1;
    else if (R) Q <= 1'b0;
    else if (S) Q <= 1'b1;
    else if (CE) begin
      if (C) begin
        Q <= D1;
        d2_held <= D2;
      end else begin
        Q <= (DDR_CLK_EDGE == "SAME_EDGE") ? d2_held : D2;
      end
    end
  end
endmodule

// Input double data rate. OPPOSITE_EDGE: Q1 takes D at the rising edge, Q2
// at the falling. SAME_EDGE: Q2's falling-edge value is presented at the
// next rising edge. SAME_EDGE_PIPELINED: Q1 is delayed to match.
module IDDR #(parameter DDR_CLK_EDGE = "OPPOSITE_EDGE", parameter INIT_Q1 = 1'b0,
              parameter INIT_Q2 = 1'b0, parameter SRTYPE = "SYNC")
    (output reg Q1, output reg Q2, input wire C, input wire CE, input wire D,
     input wire R, input wire S);
  reg fall, rise;
  initial begin
    Q1 = INIT_Q1;
    Q2 = INIT_Q2;
  end
  // One block for both edges, so that each output has one writer.
  always @(posedge C or negedge C) begin
    if (R) begin
      Q1 <= 1'b0;
      Q2 <= 1'b0;
    end else if (S) begin
      Q1 <= 1'b1;
      Q2 <= 1'b1;
    end else if (CE) begin
      if (C) begin
        rise <= D;
        Q1 <= (DDR_CLK_EDGE == "SAME_EDGE_PIPELINED") ? rise : D;
        if (DDR_CLK_EDGE != "OPPOSITE_EDGE") Q2 <= fall;
      end else begin
        fall <= D;
        if (DDR_CLK_EDGE == "OPPOSITE_EDGE") Q2 <= D;
      end
    end
  end
endmodule

// The XADC as a register file on its DRP: the configuration registers 0x40
// to 0x5F start from INIT_40 ... INIT_5F and take writes; the status
// registers read 25 degrees Celsius, VCCINT 1.0 V, VCCAUX 1.8 V and VCCBRAM
// 1.0 V in UG480's transfer functions; DRDY follows DEN by one DCLK cycle.
// No conversions run, so BUSY, EOC and EOS stay low and no alarm is raised.
module XADC #(
    parameter [15:0] INIT_40 = 0, INIT_41 = 0, INIT_42 = 16'h0800, INIT_43 = 0,
    INIT_44 = 0, INIT_45 = 0, INIT_46 = 0, INIT_47 = 0, INIT_48 = 0, INIT_49 = 0,
    INIT_4A = 0, INIT_4B = 0, INIT_4C = 0, INIT_4D = 0, INIT_4E = 0, INIT_4F = 0,
    INIT_50 = 0, INIT_51 = 0, INIT_52 = 0, INIT_53 = 0, INIT_54 = 0, INIT_55 = 0,
    INIT_56 = 0, INIT_57 = 0, INIT_58 = 0, INIT_59 = 0, INIT_5A = 0, INIT_5B = 0,
    INIT_5C = 0, INIT_5D = 0, INIT_5E = 0, INIT_5F = 0,
    parameter SIM_DEVICE = "7SERIES", parameter SIM_MONITOR_FILE = "design.txt",
    parameter IS_CONVSTCLK_INVERTED = 1'b0, parameter IS_DCLK_INVERTED = 1'b0
) (
    output wire [7:0] ALM, output wire BUSY, output wire [4:0] CHANNEL,
    output reg [15:0] DO, output reg DRDY, output wire EOC, output wire EOS,
    output wire JTAGBUSY, output wire JTAGLOCKED, output wire JTAGMODIFIED,
    output wire [4:0] MUXADDR, output wire OT,
    input wire CONVST, input wire CONVSTCLK, input wire [6:0] DADDR, input wire DCLK,
    input wire DEN, input wire [15:0] DI, input wire DWE, input wire RESET,
    input wire [15:0] VAUXN, input wire [15:0] VAUXP, input wire VN, input wire VP
);
  reg [15:0] regs [0:127];
  integer i;
  initial begin
    for (i = 0; i < 128; i = i + 1) regs[i] = 16'h0000;
    // Code = value / full scale * 4096, left-justified in 16 bits.
    regs[7'h00] = 16'h9800;  // (25 + 273.15) / 503.975 * 4096 = 2422 -> 0x976, rounded up a step
    regs[7'h01] = 16'h5550;  // 1.0 V / 3 V * 4096 = 1365
    regs[7'h02] = 16'h9990;  // 1.8 V / 3 V * 4096 = 2457
    regs[7'h06] = 16'h5550;  // 1.0 V
    regs[7'h40] = INIT_40; regs[7'h41] = INIT_41; regs[7'h42] = INIT_42; regs[7'h43] = INIT_43;
    regs[7'h44] = INIT_44; regs[7'h45] = INIT_45; regs[7'h46] = INIT_46; regs[7'h47] = INIT_47;
    regs[7'h48] = INIT_48; regs[7'h49] = INIT_49; regs[7'h4A] = INIT_4A; regs[7'h4B] = INIT_4B;
    regs[7'h4C] = INIT_4C; regs[7'h4D] = INIT_4D; regs[7'h4E] = INIT_4E; regs[7'h4F] = INIT_4F;
    regs[7'h50] = INIT_50; regs[7'h51] = INIT_51; regs[7'h52] = INIT_52; regs[7'h53] = INIT_53;
    regs[7'h54] = INIT_54; regs[7'h55] = INIT_55; regs[7'h56] = INIT_56; regs[7'h57] = INIT_57;
    regs[7'h58] = INIT_58; regs[7'h59] = INIT_59; regs[7'h5A] = INIT_5A; regs[7'h5B] = INIT_5B;
    regs[7'h5C] = INIT_5C; regs[7'h5D] = INIT_5D; regs[7'h5E] = INIT_5E; regs[7'h5F] = INIT_5F;
    DO = 16'h0000;
    DRDY = 1'b0;
  end
  always @(posedge DCLK) begin
    DRDY <= DEN;
    if (DEN) begin
      DO <= regs[DADDR];
      if (DWE && DADDR >= 7'h40) regs[DADDR] <= DI;
    end
  end
  assign ALM = 8'h00;
  assign OT = 1'b0;
  assign BUSY = 1'b0;
  assign EOC = 1'b0;
  assign EOS = 1'b0;
  assign CHANNEL = 5'd0;
  assign MUXADDR = 5'd0;
  assign JTAGBUSY = 1'b0;
  assign JTAGLOCKED = 1'b0;
  assign JTAGMODIFIED = 1'b0;
endmodule

// One clock output of a clock manager: while `run` is high, rising edges at
// `start + offset + k * period`, high for `high` of each period; low when
// `run` falls. The real-valued settings arrive as their bit patterns
// (`$realtobits`), since a module port is a vector; a change of `epoch`
// restarts the output with the new settings.
module xil_clock_out (
    output reg o,
    input wire run,
    input wire [63:0] period_bits,
    input wire [63:0] high_bits,
    input wire [63:0] offset_bits,
    input wire [31:0] epoch
);
  real period, high, offset;
  reg [31:0] started;
  initial o = 1'b0;
  always begin
    o = 1'b0;
    wait (run);
    // The settings of a new epoch are computed in the instant `run` rises,
    // in no particular order with this; a picosecond later they are there.
    // Every output waits the same, so their phases are unaffected.
    #1;
    started = epoch;
    period = $bitstoreal(period_bits);
    high = $bitstoreal(high_bits);
    offset = $bitstoreal(offset_bits);
    if (period <= 0.0) begin
      // Nothing to generate: wait for new settings rather than spin.
      @(epoch or period_bits or run);
    end else begin
      if (offset > 0.0) #(offset);
      while (run && epoch == started) begin
        o = 1'b1;
        #(high);
        o = 1'b0;
        #(period - high);
      end
    end
  end
endmodule

// The shared core of MMCME2_ADV and PLLE2_ADV; see the file's header.
module xil_clock_manager #(
    parameter real MULT = 5.0,
    parameter integer DIVCLK = 1,
    parameter real DIV0 = 1.0, parameter integer DIV1 = 1, parameter integer DIV2 = 1,
    parameter integer DIV3 = 1, parameter integer DIV4 = 1, parameter integer DIV5 = 1,
    parameter integer DIV6 = 1,
    parameter real DUTY0 = 0.5, DUTY1 = 0.5, DUTY2 = 0.5, DUTY3 = 0.5, DUTY4 = 0.5,
    parameter real DUTY5 = 0.5, DUTY6 = 0.5,
    parameter real PHASE0 = 0.0, PHASE1 = 0.0, PHASE2 = 0.0, PHASE3 = 0.0,
    parameter real PHASE4 = 0.0, PHASE5 = 0.0, PHASE6 = 0.0, PHASEFB = 0.0,
    parameter [7:0] FINE = 8'h00   // bit n: output n uses fine phase; bit 7: feedback
) (
    input wire clkin, input wire rst,
    output wire [6:0] clkout, output wire clkfbout, output reg locked,
    input wire dclk, input wire den, input wire dwe, input wire [6:0] daddr,
    input wire [15:0] di, output reg [15:0] do_, output reg drdy,
    input wire psclk, input wire psen, input wire psincdec, output reg psdone
);
  // --- the input's period -------------------------------------------------
  real t_last, t_in, p;
  integer equal;
  reg run;
  reg [31:0] epoch;
  initial begin
    t_last = -1.0;
    t_in = 0.0;
    equal = 0;
    locked = 1'b0;
    run = 1'b0;
    epoch = 0;
  end
  always @(posedge clkin or posedge rst) begin
    if (rst) begin
      equal = 0;
      locked = 1'b0;
      run = 1'b0;
      t_last = -1.0;
    end else begin
      if (t_last >= 0.0) begin
        p = $realtime - t_last;
        if (t_in > 0.0 && p > t_in * 0.99 && p < t_in * 1.01) equal = equal + 1;
        else equal = 0;
        t_in = p;
      end
      t_last = $realtime;
      // A few equal periods is a lock: the outputs start on this edge.
      if (!locked && equal >= 4) begin
        epoch = epoch + 1;
        locked = 1'b1;
        run = 1'b1;
      end
    end
  end

  // --- the DRP, in XAPP888's registers -------------------------------------
  // Reals are kept as their bit patterns in 64-bit registers: an array of
  // `real` is not something Reticle's frontend has.
  reg [15:0] drp [0:127];
  reg [63:0] div_b [0:7];  // 0 to 6 the outputs' divides, 7 the multiplier
  integer divclk, k, steps;
  reg [15:0] reg16;
  // Writes a divide's two registers: HIGH_TIME and LOW_TIME in the first,
  // EDGE and NO_COUNT in the second.
  task set_divide(input integer addr, input integer d);
    begin
      drp[addr] = {4'b0000, 6'(d / 2 + d % 2), 6'(d / 2)};
      drp[addr + 1] = {8'h00, (d % 2 == 1) ? 1'b1 : 1'b0, (d == 1) ? 1'b1 : 1'b0, 6'd0};
    end
  endtask
  // The divide two registers hold.
  function integer get_divide(input [15:0] r1, input [15:0] r2);
    begin
      if (r2[6]) get_divide = 1;
      else get_divide = r1[11:6] + r1[5:0];
    end
  endfunction
  localparam integer A0 = 7'h08, A1 = 7'h0A, A2 = 7'h0C, A3 = 7'h0E, A4 = 7'h10,
                     A5 = 7'h06, A6 = 7'h12, AFB = 7'h14, ADIV = 7'h16;
  initial begin
    for (k = 0; k < 128; k = k + 1) drp[k] = 16'h0000;
    set_divide(A0, $rtoi(DIV0));
    set_divide(A1, DIV1);
    set_divide(A2, DIV2);
    set_divide(A3, DIV3);
    set_divide(A4, DIV4);
    set_divide(A5, DIV5);
    set_divide(A6, DIV6);
    set_divide(AFB, $rtoi(MULT));
    drp[ADIV] = {2'b00, (DIVCLK % 2 == 1) ? 1'b1 : 1'b0, (DIVCLK == 1) ? 1'b1 : 1'b0,
                 6'(DIVCLK / 2 + DIVCLK % 2), 6'(DIVCLK / 2)};
    divclk = DIVCLK;
    div_b[0] = $realtobits(DIV0);
    div_b[1] = $realtobits(1.0 * DIV1);
    div_b[2] = $realtobits(1.0 * DIV2);
    div_b[3] = $realtobits(1.0 * DIV3);
    div_b[4] = $realtobits(1.0 * DIV4);
    div_b[5] = $realtobits(1.0 * DIV5);
    div_b[6] = $realtobits(1.0 * DIV6);
    div_b[7] = $realtobits(MULT);
    steps = 0;
    do_ = 16'h0000;
    drdy = 1'b0;
    psdone = 1'b0;
  end
  // Integral divides are what the registers express; a fractional one set
  // by parameter stays until a write replaces it.
  always @(posedge dclk) begin
    drdy <= den;
    if (den) begin
      do_ <= drp[daddr];
      if (dwe) begin
        drp[daddr] = di;
        case (daddr)
          A0, A0 + 1: div_b[0] = $realtobits(1.0 * get_divide(drp[A0], drp[A0 + 1]));
          A1, A1 + 1: div_b[1] = $realtobits(1.0 * get_divide(drp[A1], drp[A1 + 1]));
          A2, A2 + 1: div_b[2] = $realtobits(1.0 * get_divide(drp[A2], drp[A2 + 1]));
          A3, A3 + 1: div_b[3] = $realtobits(1.0 * get_divide(drp[A3], drp[A3 + 1]));
          A4, A4 + 1: div_b[4] = $realtobits(1.0 * get_divide(drp[A4], drp[A4 + 1]));
          A5, A5 + 1: div_b[5] = $realtobits(1.0 * get_divide(drp[A5], drp[A5 + 1]));
          A6, A6 + 1: div_b[6] = $realtobits(1.0 * get_divide(drp[A6], drp[A6 + 1]));
          AFB, AFB + 1: div_b[7] = $realtobits(1.0 * get_divide(drp[AFB], drp[AFB + 1]));
          ADIV: begin
            // A part-select of a memory element is not something Reticle's
            // frontend has; the element is copied out first.
            reg16 = drp[ADIV];
            divclk = reg16[12] ? 1 : reg16[11:6] + reg16[5:0];
          end
          default: ;
        endcase
      end
    end
  end

  // --- phase shift: PSDONE twelve PSCLK cycles after PSEN -------------------
  integer ps_wait;
  initial ps_wait = 0;
  always @(posedge psclk) begin
    psdone <= 1'b0;
    if (psen && ps_wait == 0) begin
      ps_wait = 12;
      steps = steps + (psincdec ? 1 : -1);
    end else if (ps_wait > 0) begin
      ps_wait = ps_wait - 1;
      if (ps_wait == 0) psdone <= 1'b1;
    end
  end

  // --- the outputs ----------------------------------------------------------
  // VCO period = input period * DIVCLK / multiplier; an output's period is
  // the VCO's times its divide, the feedback's the VCO's times the
  // multiplier.
  reg [63:0] per_b [0:7], hi_b [0:7], off_b [0:7], duty_b [0:7], phase_b [0:7];
  real vco, per, off;
  initial begin
    duty_b[0] = $realtobits(DUTY0); duty_b[1] = $realtobits(DUTY1);
    duty_b[2] = $realtobits(DUTY2); duty_b[3] = $realtobits(DUTY3);
    duty_b[4] = $realtobits(DUTY4); duty_b[5] = $realtobits(DUTY5);
    duty_b[6] = $realtobits(DUTY6); duty_b[7] = $realtobits(0.5);
    phase_b[0] = $realtobits(PHASE0); phase_b[1] = $realtobits(PHASE1);
    phase_b[2] = $realtobits(PHASE2); phase_b[3] = $realtobits(PHASE3);
    phase_b[4] = $realtobits(PHASE4); phase_b[5] = $realtobits(PHASE5);
    phase_b[6] = $realtobits(PHASE6); phase_b[7] = $realtobits(PHASEFB);
  end
  always @(epoch) begin
    vco = t_in * divclk / $bitstoreal(div_b[7]);
    for (k = 0; k < 8; k = k + 1) begin
      per = vco * $bitstoreal(div_b[k]);
      off = per * $bitstoreal(phase_b[k]) / 360.0;
      if (FINE[k]) off = off + steps * vco / 56.0;
      while (off < 0.0) off = off + per;
      per_b[k] = $realtobits(per);
      hi_b[k] = $realtobits(per * $bitstoreal(duty_b[k]));
      off_b[k] = $realtobits(off);
    end
  end
  genvar g;
  for (g = 0; g < 7; g = g + 1) begin : outs
    xil_clock_out u (.o(clkout[g]), .run(run), .period_bits(per_b[g]), .high_bits(hi_b[g]),
                     .offset_bits(off_b[g]), .epoch(epoch));
  end
  xil_clock_out ufb (.o(clkfbout), .run(run), .period_bits(per_b[7]), .high_bits(hi_b[7]),
                     .offset_bits(off_b[7]), .epoch(epoch));
endmodule

module MMCME2_ADV #(
    parameter BANDWIDTH = "OPTIMIZED",
    parameter real CLKFBOUT_MULT_F = 5.0,
    parameter real CLKFBOUT_PHASE = 0.0,
    parameter CLKFBOUT_USE_FINE_PS = "FALSE",
    parameter real CLKIN1_PERIOD = 0.0,
    parameter real CLKIN2_PERIOD = 0.0,
    parameter real CLKOUT0_DIVIDE_F = 1.0,
    parameter real CLKOUT0_DUTY_CYCLE = 0.5,
    parameter real CLKOUT0_PHASE = 0.0,
    parameter CLKOUT0_USE_FINE_PS = "FALSE",
    parameter integer CLKOUT1_DIVIDE = 1,
    parameter real CLKOUT1_DUTY_CYCLE = 0.5,
    parameter real CLKOUT1_PHASE = 0.0,
    parameter CLKOUT1_USE_FINE_PS = "FALSE",
    parameter integer CLKOUT2_DIVIDE = 1,
    parameter real CLKOUT2_DUTY_CYCLE = 0.5,
    parameter real CLKOUT2_PHASE = 0.0,
    parameter CLKOUT2_USE_FINE_PS = "FALSE",
    parameter integer CLKOUT3_DIVIDE = 1,
    parameter real CLKOUT3_DUTY_CYCLE = 0.5,
    parameter real CLKOUT3_PHASE = 0.0,
    parameter CLKOUT3_USE_FINE_PS = "FALSE",
    parameter CLKOUT4_CASCADE = "FALSE",
    parameter integer CLKOUT4_DIVIDE = 1,
    parameter real CLKOUT4_DUTY_CYCLE = 0.5,
    parameter real CLKOUT4_PHASE = 0.0,
    parameter CLKOUT4_USE_FINE_PS = "FALSE",
    parameter integer CLKOUT5_DIVIDE = 1,
    parameter real CLKOUT5_DUTY_CYCLE = 0.5,
    parameter real CLKOUT5_PHASE = 0.0,
    parameter CLKOUT5_USE_FINE_PS = "FALSE",
    parameter integer CLKOUT6_DIVIDE = 1,
    parameter real CLKOUT6_DUTY_CYCLE = 0.5,
    parameter real CLKOUT6_PHASE = 0.0,
    parameter CLKOUT6_USE_FINE_PS = "FALSE",
    parameter COMPENSATION = "ZHOLD",
    parameter integer DIVCLK_DIVIDE = 1,
    parameter real REF_JITTER1 = 0.0,
    parameter real REF_JITTER2 = 0.0,
    parameter SS_EN = "FALSE",
    parameter SS_MODE = "CENTER_HIGH",
    parameter integer SS_MOD_PERIOD = 10000,
    parameter STARTUP_WAIT = "FALSE"
) (
    output wire CLKFBOUT, output wire CLKFBOUTB, output wire CLKFBSTOPPED,
    output wire CLKINSTOPPED, output wire CLKOUT0, output wire CLKOUT0B,
    output wire CLKOUT1, output wire CLKOUT1B, output wire CLKOUT2, output wire CLKOUT2B,
    output wire CLKOUT3, output wire CLKOUT3B, output wire CLKOUT4, output wire CLKOUT5,
    output wire CLKOUT6, output wire [15:0] DO, output wire DRDY, output wire LOCKED,
    output wire PSDONE,
    input wire CLKFBIN, input wire CLKIN1, input wire CLKIN2, input wire CLKINSEL,
    input wire [6:0] DADDR, input wire DCLK, input wire DEN, input wire [15:0] DI,
    input wire DWE, input wire PSCLK, input wire PSEN, input wire PSINCDEC,
    input wire PWRDWN, input wire RST
);
  wire [6:0] out;
  wire locked;
  // CLKINSEL high selects CLKIN1; tied off, it is high.
  wire clkin = (CLKINSEL === 1'b0) ? CLKIN2 : CLKIN1;
  xil_clock_manager #(
      .MULT(CLKFBOUT_MULT_F), .DIVCLK(DIVCLK_DIVIDE),
      .DIV0(CLKOUT0_DIVIDE_F), .DIV1(CLKOUT1_DIVIDE), .DIV2(CLKOUT2_DIVIDE),
      .DIV3(CLKOUT3_DIVIDE), .DIV4(CLKOUT4_DIVIDE), .DIV5(CLKOUT5_DIVIDE),
      .DIV6(CLKOUT6_DIVIDE),
      .DUTY0(CLKOUT0_DUTY_CYCLE), .DUTY1(CLKOUT1_DUTY_CYCLE), .DUTY2(CLKOUT2_DUTY_CYCLE),
      .DUTY3(CLKOUT3_DUTY_CYCLE), .DUTY4(CLKOUT4_DUTY_CYCLE), .DUTY5(CLKOUT5_DUTY_CYCLE),
      .DUTY6(CLKOUT6_DUTY_CYCLE),
      .PHASE0(CLKOUT0_PHASE), .PHASE1(CLKOUT1_PHASE), .PHASE2(CLKOUT2_PHASE),
      .PHASE3(CLKOUT3_PHASE), .PHASE4(CLKOUT4_PHASE), .PHASE5(CLKOUT5_PHASE),
      .PHASE6(CLKOUT6_PHASE), .PHASEFB(CLKFBOUT_PHASE),
      .FINE({CLKFBOUT_USE_FINE_PS == "TRUE", CLKOUT6_USE_FINE_PS == "TRUE",
             CLKOUT5_USE_FINE_PS == "TRUE", CLKOUT4_USE_FINE_PS == "TRUE",
             CLKOUT3_USE_FINE_PS == "TRUE", CLKOUT2_USE_FINE_PS == "TRUE",
             CLKOUT1_USE_FINE_PS == "TRUE", CLKOUT0_USE_FINE_PS == "TRUE"})
  ) core (
      .clkin(clkin), .rst(RST | PWRDWN), .clkout(out), .clkfbout(CLKFBOUT), .locked(locked),
      .dclk(DCLK), .den(DEN), .dwe(DWE), .daddr(DADDR), .di(DI), .do_(DO), .drdy(DRDY),
      .psclk(PSCLK), .psen(PSEN), .psincdec(PSINCDEC), .psdone(PSDONE)
  );
  assign LOCKED = locked;
  assign {CLKOUT6, CLKOUT5, CLKOUT4, CLKOUT3, CLKOUT2, CLKOUT1, CLKOUT0} = out;
  assign CLKOUT0B = ~out[0];
  assign CLKOUT1B = ~out[1];
  assign CLKOUT2B = ~out[2];
  assign CLKOUT3B = ~out[3];
  assign CLKFBOUTB = ~CLKFBOUT;
  assign CLKINSTOPPED = 1'b0;
  assign CLKFBSTOPPED = 1'b0;
endmodule

module PLLE2_ADV #(
    parameter BANDWIDTH = "OPTIMIZED",
    parameter integer CLKFBOUT_MULT = 5,
    parameter real CLKFBOUT_PHASE = 0.0,
    parameter real CLKIN1_PERIOD = 0.0,
    parameter real CLKIN2_PERIOD = 0.0,
    parameter integer CLKOUT0_DIVIDE = 1,
    parameter real CLKOUT0_DUTY_CYCLE = 0.5,
    parameter real CLKOUT0_PHASE = 0.0,
    parameter integer CLKOUT1_DIVIDE = 1,
    parameter real CLKOUT1_DUTY_CYCLE = 0.5,
    parameter real CLKOUT1_PHASE = 0.0,
    parameter integer CLKOUT2_DIVIDE = 1,
    parameter real CLKOUT2_DUTY_CYCLE = 0.5,
    parameter real CLKOUT2_PHASE = 0.0,
    parameter integer CLKOUT3_DIVIDE = 1,
    parameter real CLKOUT3_DUTY_CYCLE = 0.5,
    parameter real CLKOUT3_PHASE = 0.0,
    parameter integer CLKOUT4_DIVIDE = 1,
    parameter real CLKOUT4_DUTY_CYCLE = 0.5,
    parameter real CLKOUT4_PHASE = 0.0,
    parameter integer CLKOUT5_DIVIDE = 1,
    parameter real CLKOUT5_DUTY_CYCLE = 0.5,
    parameter real CLKOUT5_PHASE = 0.0,
    parameter COMPENSATION = "ZHOLD",
    parameter integer DIVCLK_DIVIDE = 1,
    parameter real REF_JITTER1 = 0.0,
    parameter real REF_JITTER2 = 0.0,
    parameter STARTUP_WAIT = "FALSE"
) (
    output wire CLKFBOUT, output wire CLKOUT0, output wire CLKOUT1, output wire CLKOUT2,
    output wire CLKOUT3, output wire CLKOUT4, output wire CLKOUT5,
    output wire [15:0] DO, output wire DRDY, output wire LOCKED,
    input wire CLKFBIN, input wire CLKIN1, input wire CLKIN2, input wire CLKINSEL,
    input wire [6:0] DADDR, input wire DCLK, input wire DEN, input wire [15:0] DI,
    input wire DWE, input wire PWRDWN, input wire RST
);
  wire [6:0] out;
  wire clkin = (CLKINSEL === 1'b0) ? CLKIN2 : CLKIN1;
  wire psdone_unused;
  xil_clock_manager #(
      .MULT(CLKFBOUT_MULT), .DIVCLK(DIVCLK_DIVIDE),
      .DIV0(CLKOUT0_DIVIDE), .DIV1(CLKOUT1_DIVIDE), .DIV2(CLKOUT2_DIVIDE),
      .DIV3(CLKOUT3_DIVIDE), .DIV4(CLKOUT4_DIVIDE), .DIV5(CLKOUT5_DIVIDE),
      .DUTY0(CLKOUT0_DUTY_CYCLE), .DUTY1(CLKOUT1_DUTY_CYCLE), .DUTY2(CLKOUT2_DUTY_CYCLE),
      .DUTY3(CLKOUT3_DUTY_CYCLE), .DUTY4(CLKOUT4_DUTY_CYCLE), .DUTY5(CLKOUT5_DUTY_CYCLE),
      .PHASE0(CLKOUT0_PHASE), .PHASE1(CLKOUT1_PHASE), .PHASE2(CLKOUT2_PHASE),
      .PHASE3(CLKOUT3_PHASE), .PHASE4(CLKOUT4_PHASE), .PHASE5(CLKOUT5_PHASE),
      .PHASEFB(CLKFBOUT_PHASE)
  ) core (
      .clkin(clkin), .rst(RST | PWRDWN), .clkout(out), .clkfbout(CLKFBOUT), .locked(LOCKED),
      .dclk(DCLK), .den(DEN), .dwe(DWE), .daddr(DADDR), .di(DI), .do_(DO), .drdy(DRDY),
      .psclk(1'b0), .psen(1'b0), .psincdec(1'b0), .psdone(psdone_unused)
  );
  assign {CLKOUT5, CLKOUT4, CLKOUT3, CLKOUT2, CLKOUT1, CLKOUT0} = out[5:0];
endmodule
