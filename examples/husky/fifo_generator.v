// Stand-ins for the six Vivado FIFO Generator cores NewAE's
// ChipWhisperer-Husky gateware instantiates. Vivado builds each from the
// block RAM's own FIFO logic (`Common_Clock_Builtin_FIFO` or
// `Independent_Clocks_Builtin_FIFO`); here each is an ordinary memory with
// the pointers and flags in the fabric, so that Reticle infers the block
// RAMs and builds the rest from logic.
//
// What each wrapper's parameters come from: the core's `.xci` in
// chipwhisperer-husky-fpga 2dfe1bc, `fpga/vivado/cwhusky.srcs/sources_1/ip/`
// — `Input_Data_Width`, `Input_Depth`, `Performance_Options`, the
// `*_Threshold_Assert_Value` of each programmable flag, and which of
// `overflow` and `underflow` exist. Those are quoted. The behaviour of
// the flags is the FIFO Generator's as PG057 describes it, and is a
// reading:
//
//   * `empty` and `full` are exact: no word is lost or invented, and
//     reading an empty FIFO or writing a full one changes nothing.
//   * `overflow` and `underflow` are a write refused because the FIFO was
//     full, and a read refused because it was empty, one cycle later, in
//     the clock of the port refused.
//   * `prog_full` is "at least the threshold words held", in the write
//     clock; `prog_empty` is "at most the threshold words held", in the
//     read clock. With independent clocks each side counts against the
//     other side's pointer after synchronisation, so it lags.
//   * First-word fall-through: the oldest word is on `dout` whenever
//     `empty` is low, and `rd_en` takes it away. Standard: `rd_en`
//     brings the next word to `dout` one cycle later.
//   * `rst` is asynchronous, active high. While it is held `empty` is
//     high and `full` low (`Full_Flags_Reset_Value = 0`).
//
// What is deliberately not the same: in fall-through mode this holds the
// depth plus the one word waiting on `dout`, and the built-in FIFO's own
// figure is not reproduced; the cycle on which a flag moves after a write
// on the other clock is not the hardware's either. NewAE's logic
// around these cores counts samples itself and reads the flags as levels,
// which is what makes a stand-in reasonable — that is a reading of their
// source, not a measurement.

// One FIFO. `AW` is log2 of the depth; `FWFT` selects fall-through;
// `ASYNC` selects independent clocks, with `rd_clk` used only then.
// `PROG_FULL` and `PROG_EMPTY` are thresholds in words; a flag the core
// does not have is left unconnected by its wrapper.
module husky_fifo #(
    parameter WIDTH = 8,
    parameter AW = 10,
    parameter FWFT = 0,
    parameter ASYNC = 0,
    parameter PROG_FULL = 0,
    parameter PROG_EMPTY = 0
) (
    input  wire             rst,
    input  wire             wr_clk,
    input  wire             rd_clk,
    input  wire [WIDTH-1:0] din,
    input  wire             wr_en,
    input  wire             rd_en,
    output wire [WIDTH-1:0] dout,
    output wire             full,
    output wire             empty,
    output wire             prog_full,
    output wire             prog_empty,
    output reg              overflow,
    output reg              underflow
);
  localparam DEPTH = 1 << AW;

  wire rclk = ASYNC ? rd_clk : wr_clk;

  // Resets: asserted at once, released on each clock's own edge.
  reg [1:0] wrst_sync, rrst_sync;
  always @(posedge wr_clk or posedge rst)
    if (rst) wrst_sync <= 2'b11;
    else wrst_sync <= {wrst_sync[0], 1'b0};
  always @(posedge rclk or posedge rst)
    if (rst) rrst_sync <= 2'b11;
    else rrst_sync <= {rrst_sync[0], 1'b0};
  wire wrst = wrst_sync[1];
  wire rrst = rrst_sync[1];

  (* ram_style = "block" *) reg [WIDTH-1:0] mem [0:DEPTH-1];

  // Pointers one bit wider than the address: equal is empty, equal but
  // for the top bit is full.
  reg  [AW:0] wptr, rptr;
  // A word already read out of the memory and waiting on `dout`: in
  // fall-through mode, the word `empty` low promises.
  reg         held;
  wire [AW:0] rptr_w;  // the read pointer as the write side sees it
  wire [AW:0] wptr_r;  // the write pointer as the read side sees it

  // --- write side ----------------------------------------------------------
  wire [AW:0] wused = wptr - rptr_w;
  assign full = !wrst && wused[AW];
  wire do_write = wr_en && !full && !wrst;
  always @(posedge wr_clk) if (do_write) mem[wptr[AW-1:0]] <= din;
  always @(posedge wr_clk or posedge rst)
    if (rst) begin
      wptr <= 0;
      overflow <= 1'b0;
    end else begin
      if (do_write) wptr <= wptr + 1'b1;
      overflow <= wr_en && full;
    end
  // With one clock the word waiting on `dout` in fall-through mode is
  // still in the FIFO, and counts; with two the write side cannot see it.
  wire [AW+1:0] wcount = wused + ((FWFT && !ASYNC) ? held : 1'b0);
  assign prog_full = !wrst && (wcount >= PROG_FULL);

  // --- read side -----------------------------------------------------------
  reg  [WIDTH-1:0] q;
  wire             stored = (wptr_r != rptr);  // words still in the memory
  wire             take;  // a read the user asked for and gets
  wire             fetch;  // the memory read this cycle
  if (FWFT) begin : fall_through
    assign empty = !held;
    assign take  = rd_en && held;
    assign fetch = stored && (!held || take) && !rrst;
  end else begin : standard
    assign empty = !stored || rrst;
    assign take  = rd_en && !empty;
    assign fetch = take;
  end
  always @(posedge rclk) if (fetch) q <= mem[rptr[AW-1:0]];
  always @(posedge rclk or posedge rst)
    if (rst) begin
      rptr <= 0;
      held <= 1'b0;
      underflow <= 1'b0;
    end else begin
      if (fetch) rptr <= rptr + 1'b1;
      held <= fetch || (held && !take);
      underflow <= rd_en && empty;
    end
  assign dout = q;
  wire [AW:0] rused = (wptr_r - rptr) + (FWFT ? held : 1'b0);
  assign prog_empty = rrst || (rused <= PROG_EMPTY);

  // --- crossing ------------------------------------------------------------
  if (ASYNC) begin : crossing
    // Gray code, two flip-flops on each side.
    wire [AW:0] wgray = wptr ^ (wptr >> 1);
    wire [AW:0] rgray = rptr ^ (rptr >> 1);
    (* ASYNC_REG = "TRUE" *) reg [AW:0] wgray_r1, wgray_r2;
    (* ASYNC_REG = "TRUE" *) reg [AW:0] rgray_w1, rgray_w2;
    always @(posedge rclk or posedge rst)
      if (rst) {wgray_r2, wgray_r1} <= 0;
      else {wgray_r2, wgray_r1} <= {wgray_r1, wgray};
    always @(posedge wr_clk or posedge rst)
      if (rst) {rgray_w2, rgray_w1} <= 0;
      else {rgray_w2, rgray_w1} <= {rgray_w1, rgray};
    assign wptr_r = gray_to_binary(wgray_r2);
    assign rptr_w = gray_to_binary(rgray_w2);
  end else begin : common
    assign wptr_r = wptr;
    assign rptr_w = rptr;
  end

  function [AW:0] gray_to_binary(input [AW:0] g);
    integer i;
    begin
      gray_to_binary[AW] = g[AW];
      for (i = AW - 1; i >= 0; i = i - 1) gray_to_binary[i] = gray_to_binary[i+1] ^ g[i];
    end
  endfunction
endmodule

// 12 x 2048, common clock, standard. `adc_fast_fifo.xci`.
module adc_fast_fifo (
    input  wire        clk,
    input  wire        rst,
    input  wire [11:0] din,
    input  wire        wr_en,
    input  wire        rd_en,
    output wire [11:0] dout,
    output wire        full,
    output wire        empty,
    output wire        overflow,
    output wire        underflow
);
  husky_fifo #(
      .WIDTH(12),
      .AW(11)
  ) u_fifo (
      .rst(rst), .wr_clk(clk), .rd_clk(clk), .din(din), .wr_en(wr_en), .rd_en(rd_en),
      .dout(dout), .full(full), .empty(empty), .prog_full(), .prog_empty(),
      .overflow(overflow), .underflow(underflow)
  );
endmodule

// 72 x 4096, independent clocks, fall-through, `prog_empty` at 6.
// `fast_slower_fifo.xci`.
module fast_slower_fifo (
    input  wire        rst,
    input  wire        wr_clk,
    input  wire        rd_clk,
    input  wire [71:0] din,
    input  wire        wr_en,
    input  wire        rd_en,
    output wire [71:0] dout,
    output wire        full,
    output wire        empty,
    output wire        prog_empty,
    output wire        overflow,
    output wire        underflow
);
  husky_fifo #(
      .WIDTH(72),
      .AW(12),
      .FWFT(1),
      .ASYNC(1),
      .PROG_EMPTY(6)
  ) u_fifo (
      .rst(rst), .wr_clk(wr_clk), .rd_clk(rd_clk), .din(din), .wr_en(wr_en), .rd_en(rd_en),
      .dout(dout), .full(full), .empty(empty), .prog_full(), .prog_empty(prog_empty),
      .overflow(overflow), .underflow(underflow)
  );
endmodule

// 18 x 8192, independent clocks, fall-through, `prog_full` at 8188 and
// `prog_empty` at 128, no `overflow`. `fifo_generator_0.xci`.
module fifo_generator_0 (
    input  wire        rst,
    input  wire        wr_clk,
    input  wire        rd_clk,
    input  wire [17:0] din,
    input  wire        wr_en,
    input  wire        rd_en,
    output wire [17:0] dout,
    output wire        full,
    output wire        empty,
    output wire        prog_full,
    output wire        prog_empty,
    output wire        underflow
);
  husky_fifo #(
      .WIDTH(18),
      .AW(13),
      .FWFT(1),
      .ASYNC(1),
      .PROG_FULL(8188),
      .PROG_EMPTY(128)
  ) u_fifo (
      .rst(rst), .wr_clk(wr_clk), .rd_clk(rd_clk), .din(din), .wr_en(wr_en), .rd_en(rd_en),
      .dout(dout), .full(full), .empty(empty), .prog_full(prog_full),
      .prog_empty(prog_empty), .overflow(), .underflow(underflow)
  );
endmodule

// 32 x 1024, common clock, standard, no `overflow`. `trigger_fifo.xci`.
module trigger_fifo (
    input  wire        clk,
    input  wire        rst,
    input  wire [31:0] din,
    input  wire        wr_en,
    input  wire        rd_en,
    output wire [31:0] dout,
    output wire        full,
    output wire        empty,
    output wire        underflow
);
  husky_fifo #(
      .WIDTH(32),
      .AW(10)
  ) u_fifo (
      .rst(rst), .wr_clk(clk), .rd_clk(clk), .din(din), .wr_en(wr_en), .rd_en(rd_en),
      .dout(dout), .full(full), .empty(empty), .prog_full(), .prog_empty(),
      .overflow(), .underflow(underflow)
  );
endmodule

// 72 x 2048, common clock, fall-through, `prog_full` at 2047 and
// `prog_empty` at 4. `usb_slow_fifo1.xci`.
module usb_slow_fifo1 (
    input  wire        clk,
    input  wire        rst,
    input  wire [71:0] din,
    input  wire        wr_en,
    input  wire        rd_en,
    output wire [71:0] dout,
    output wire        full,
    output wire        empty,
    output wire        prog_full,
    output wire        prog_empty,
    output wire        overflow,
    output wire        underflow
);
  husky_fifo #(
      .WIDTH(72),
      .AW(11),
      .FWFT(1),
      .PROG_FULL(2047),
      .PROG_EMPTY(4)
  ) u_fifo (
      .rst(rst), .wr_clk(clk), .rd_clk(clk), .din(din), .wr_en(wr_en), .rd_en(rd_en),
      .dout(dout), .full(full), .empty(empty), .prog_full(prog_full),
      .prog_empty(prog_empty), .overflow(overflow), .underflow(underflow)
  );
endmodule

// 72 x 16384, common clock, fall-through, `prog_full` at 16384.
// `usb_slow_fifo2.xci`.
module usb_slow_fifo2 (
    input  wire        clk,
    input  wire        rst,
    input  wire [71:0] din,
    input  wire        wr_en,
    input  wire        rd_en,
    output wire [71:0] dout,
    output wire        full,
    output wire        empty,
    output wire        prog_full,
    output wire        overflow,
    output wire        underflow
);
  husky_fifo #(
      .WIDTH(72),
      .AW(14),
      .FWFT(1),
      .PROG_FULL(16384)
  ) u_fifo (
      .rst(rst), .wr_clk(clk), .rd_clk(clk), .din(din), .wr_en(wr_en), .rd_en(rd_en),
      .dout(dout), .full(full), .empty(empty), .prog_full(prog_full),
      .prog_empty(), .overflow(overflow), .underflow(underflow)
  );
endmodule
