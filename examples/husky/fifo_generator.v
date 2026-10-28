// Stand-ins for the six Vivado FIFO Generator cores NewAE's
// ChipWhisperer-Husky gateware instantiates. Vivado builds each from the
// block RAM's own FIFO logic (`Common_Clock_Builtin_FIFO` or
// `Independent_Clocks_Builtin_FIFO`); here each is an ordinary memory with
// the pointers and flags in the fabric, so that Reticle infers the block
// RAMs and builds the rest from logic.
//
// What each wrapper's parameters come from: the core's `.xci` in
// chipwhisperer-husky-fpga 2dfe1bc, `fpga/vivado/cwhusky.srcs/sources_1/ip/`,
// and with `PLUS` defined `cwhusky_plus.srcs`'s, which makes three of them
// deeper — `Input_Data_Width`, `Input_Depth`, `Performance_Options`, the
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
// figure is not reproduced, except where NewAE's logic needs more: the
// Husky Plus's two slow FIFOs set `prog_full` past their depth, and hold
// two words past that threshold here (`extra_words`), which is a reading
// of the thresholds and not a measurement of the core. The cycle on which
// a flag moves after a write on the other clock is not the hardware's
// either. NewAE's logic
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
    parameter PROG_EMPTY = 0,
    // Words per bank, as a power of two; at `AW`, one memory. See `banked`.
    parameter BANK_AW = AW,
    // Fall-through only: words held beyond the depth and the word on
    // `dout`, at most 31. See `extra_words`.
    parameter EXTRA = 0
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

  localparam BANKED = BANK_AW < AW;

  // Pointers one bit wider than the address: equal is empty, equal but
  // for the top bit is full.
  reg  [AW:0] wptr, rptr;
  // A word already read out of the memory and waiting on `dout`: in
  // fall-through mode, the word `empty` low promises.
  reg         held;
  wire [AW:0] rptr_w;  // the read pointer as the write side sees it
  wire [AW:0] wptr_r;  // the write pointer as the read side sees it
  // And each as it will be after this cycle's edge. The flags below are
  // registers, set each cycle to what the pointers will make them: the
  // value the same expression of the pointers would have the cycle after,
  // without its subtraction and comparison in front of every write and
  // read enable of the memory. Combinational, they were the Husky Plus's
  // latest paths in `clk_usb`, from a pointer through `full` to the
  // enables of the 64 block RAMs of the largest FIFO, at 14 ns of 10.4.
  wire [AW:0] rptr_w_next;
  wire [AW:0] wptr_r_next;
  wire [AW:0] wptr_next;
  wire [AW:0] rptr_next;
  wire        held_next;
  // Words read out of the memory and not yet taken, after this edge: the
  // word on `dout`, and in `extra_words` those queued behind it.
  wire [5:0]  out_next;
  wire [WIDTH-1:0] mem_q;  // the memory's read register
  // In `extra_words`, `stage` and whether it holds a word, for the
  // testbench to check the two cycles it is given; nothing reads them.
  wire             staged_word;
  wire [WIDTH-1:0] stage_word;

  // --- write side ----------------------------------------------------------
  wire [AW:0] wused_next = wptr_next - rptr_w_next;
  reg full_q;
  assign full = !wrst && full_q;
  wire do_write = wr_en && !full && !wrst;
  assign wptr_next = wptr + do_write;
  always @(posedge wr_clk or posedge rst)
    if (rst) begin
      wptr <= 0;
      full_q <= 1'b0;
      prog_full_q <= (PROG_FULL == 0);
      overflow <= 1'b0;
    end else begin
      wptr <= wptr_next;
      full_q <= wused_next[AW];
      prog_full_q <= wcount_next >= PROG_FULL;
      overflow <= wr_en && full;
    end
  // With one clock the word waiting on `dout` in fall-through mode is
  // still in the FIFO, and counts; with two the write side cannot see it.
  wire [AW+1:0] wcount_next = wused_next + ((FWFT && !ASYNC) ? out_next : 6'd0);
  reg prog_full_q;
  assign prog_full = !wrst && prog_full_q;

  // --- read side -----------------------------------------------------------
  reg  [WIDTH-1:0] q;
  reg              stored;  // words still in the memory
  wire             take;  // a read the user asked for and gets
  wire             fetch;  // the memory read this cycle
  if (FWFT && EXTRA == 0) begin : fall_through
    assign empty = !held;
    assign take  = rd_en && held;
    assign fetch = stored && (!held || take) && !rrst;
    assign held_next = fetch || (held && !take);
    assign staged_word = 1'b0;
    assign stage_word = {WIDTH{1'b0}};
    assign out_next = {5'd0, held_next};
    assign dout = mem_q;
  end else if (FWFT) begin : extra_words
    // The FIFO Generator's built-in FIFO holds more than its depth: the
    // Husky Plus's `usb_slow_fifo2` is 32,768 deep with `prog_full` at
    // 32,782, and NewAE's `slow_fifo_wrapper` moves words into it until
    // that flag rises, without looking at `full`. Holding only the depth,
    // it overflowed: on the part, 12-bit captures of 195,000 samples were
    // exact and of 197,000 failed with "slow FIFO overflow", the boundary
    // being its 32,769 words. Here the memory's words come out through a
    // register, then a queue of `EXTRA` in distributed RAM, to a register
    // on `dout`. The first register is for the part: written straight from
    // the memory, through the multiplexer choosing a bank, the queue's
    // inputs were `clk_usb`'s late paths, at 12.5 ns of 10.4, and captures
    // read back bits set that the ramp had clear, some in every word.
    //
    // And that register's own input came back late, build after build:
    // the multiplexer that chooses a bank, and the one Reticle's block RAM
    // mapping puts behind every bank of eight blocks deep, gather a word
    // from block RAMs spread over half the die, and on one build of four
    // or so the path was past 12 ns of 10.4 and the ramps read back wrong
    // again. Splitting the memory to need no multiplexer, or registering
    // each block's word beside it, crowded the block RAM columns until
    // the route did not settle. So the memory is read at most every other
    // cycle, which leaves the word on `mem_q` for two, and `stage` takes
    // it on the second: `husky.rcf` gives the paths into `stage` two
    // periods. Every other cycle of `clk_usb` is 48 million words a
    // second, past the 41.7 million a 250 MHz ADC writes and the few
    // million USB reads.
    reg  [5:0] outside;  // `held`, the queue, `stage` and the pipeline
    reg        fetched;  // `mem_q` holds the word read last cycle
    reg        piped;  // and still holds it
    reg        staged;  // `stage` holds the word before it
    reg  [WIDTH-1:0] stage;
    reg  [5:0] queued;
    reg  [4:0] qw, qr;
    reg  [WIDTH-1:0] head;
    (* ram_style = "distributed" *) reg [WIDTH-1:0] queue [0:31];
    wire free = !held || take;  // `head` may be loaded
    wire from_queue = free && queued != 0;
    wire from_stage = free && queued == 0 && staged;
    wire push = staged && !from_stage;
    assign empty = !held;
    assign take  = rd_en && held;
    assign fetch = stored && !rrst && outside - take <= EXTRA && !fetched;
    assign held_next = from_queue || from_stage || (held && !take);
    assign out_next = outside - take + fetch;
    assign dout = head;
    assign staged_word = staged;
    assign stage_word = stage;
    always @(posedge rclk) begin
      stage <= mem_q;  // right only when `piped`; see above
      if (push) queue[qw] <= stage;
      if (from_queue) head <= queue[qr];
      else if (from_stage) head <= stage;
    end
    always @(posedge rclk or posedge rst)
      if (rst) begin
        outside <= 0;
        fetched <= 1'b0;
        piped <= 1'b0;
        staged <= 1'b0;
        queued <= 0;
        qw <= 0;
        qr <= 0;
      end else begin
        outside <= out_next;
        fetched <= fetch;
        piped <= fetched;
        staged <= piped;
        queued <= queued + push - from_queue;
        qw <= qw + push;
        qr <= qr + from_queue;
      end
  end else begin : standard
    assign empty = !stored || rrst;
    assign take  = rd_en && !empty;
    assign fetch = take;
    assign held_next = 1'b0;
    assign out_next = 6'd0;
    assign dout = mem_q;
    assign staged_word = 1'b0;
    assign stage_word = {WIDTH{1'b0}};
  end
  assign rptr_next = rptr + fetch;
  wire [AW:0] rused_next = (wptr_r_next - rptr_next) + out_next;
  reg prog_empty_q;
  always @(posedge rclk or posedge rst)
    if (rst) begin
      rptr <= 0;
      held <= 1'b0;
      stored <= 1'b0;
      prog_empty_q <= 1'b1;
      underflow <= 1'b0;
    end else begin
      rptr <= rptr_next;
      held <= held_next;
      stored <= wptr_r_next != rptr_next;
      prog_empty_q <= rused_next <= PROG_EMPTY;
      underflow <= rd_en && empty;
    end
  assign prog_empty = rrst || prog_empty_q;

  // --- memory ----------------------------------------------------------------
  // The deepest FIFOs fill half the die's block RAMs, and one pointer and
  // one `din` reaching every one of them were the Husky Plus's latest
  // `clk_usb` paths: a write address at 15.8 ns of 10.4, the write data
  // 16.8 ns when registered once in front of the memory. Banked, each bank
  // has registers of its own, which the placer can seat beside it: a copy
  // of the read pointer, kept equal to it, and the write a cycle after it
  // is taken, so that the read side sees a write a cycle later (below).
  //
  // The slow FIFOs' banks are 8192 words. Checked on the Husky Plus on
  // 2026-10-25: so banked, its late `clk_usb` endpoints in `oadc` went
  // from 1,787 to 145, and test ramps at 8 and 12 bits were exact up to
  // 131,070 samples six times in six, where unbanked 12-bit captures of
  // 30,000 were exact one time in three. Banks of 4096, twice the
  // registers, crowded the block RAM columns: the route was still 850
  // nodes overused after 27 passes and 77 minutes, and was stopped.
  //
  // Each bank is segments of 1024 words, four block RAMs of 1024 by 18
  // each, and the word read is the OR of every segment's output gated by
  // a register of the segment's own that says the word is its: no
  // multiplexer chooses a bank or a block. As one memory a bank was
  // mapped eight blocks deep behind a multiplexer of the mapping's own,
  // whose select register sat where the placer left it: on one build
  // 13 ns of wire from the multiplexer it drove, at the far side of the
  // die from it, past even the two periods `stage` has (see
  // `extra_words`). Each segment's register is beside its block RAMs, set
  // from its bank's copy of the read address.
  if (BANKED) begin : banked
    localparam NB = 1 << (AW - BANK_AW);
    localparam SEG_AW = BANK_AW < 10 ? BANK_AW : 10;
    localparam NS = 1 << (BANK_AW - SEG_AW);
    wire [NB*NS*WIDTH-1:0] seg_q;  // each segment's word, zero if not its
    genvar b, g;
    for (b = 0; b < NB; b = b + 1) begin : bank
      (* keep = "true" *) reg [BANK_AW-1:0] waddr;
      (* keep = "true" *) reg [WIDTH-1:0]   wdata;
      (* keep = "true" *) reg               wen;
      (* keep = "true" *) reg [BANK_AW-1:0] raddr;
      (* keep = "true" *) reg               rbank;  // `raddr` is in this bank
      always @(posedge wr_clk) begin
        waddr <= wptr[BANK_AW-1:0];
        wdata <= din;
      end
      always @(posedge wr_clk or posedge rst)
        if (rst) wen <= 1'b0;
        else wen <= do_write && wptr[AW-1:BANK_AW] == b;
      always @(posedge rclk or posedge rst)
        if (rst) begin
          raddr <= 0;
          rbank <= b == 0;
        end else begin
          raddr <= rptr_next[BANK_AW-1:0];
          rbank <= rptr_next[AW-1:BANK_AW] == b;
        end
      for (g = 0; g < NS; g = g + 1) begin : segment
        (* ram_style = "block" *) reg [WIDTH-1:0] mem [0:(1 << SEG_AW)-1];
        reg [WIDTH-1:0] q;
        reg             mine;  // `q` is the word read
        always @(posedge wr_clk)
          if (wen && waddr[BANK_AW-1:SEG_AW] == g) mem[waddr[SEG_AW-1:0]] <= wdata;
        always @(posedge rclk)
          if (fetch) begin
            q <= mem[raddr[SEG_AW-1:0]];
            mine <= rbank && raddr[BANK_AW-1:SEG_AW] == g;
          end
        assign seg_q[(b*NS+g)*WIDTH+:WIDTH] = q & {WIDTH{mine}};
      end
    end
    reg [WIDTH-1:0] any_segment;
    integer k;
    always @* begin
      any_segment = {WIDTH{1'b0}};
      for (k = 0; k < NB * NS; k = k + 1) any_segment = any_segment | seg_q[k*WIDTH+:WIDTH];
    end
    assign mem_q = any_segment;
  end else begin : single
    (* ram_style = "block" *) reg [WIDTH-1:0] mem [0:DEPTH-1];
    always @(posedge wr_clk) if (do_write) mem[wptr[AW-1:0]] <= din;
    always @(posedge rclk) if (fetch) q <= mem[rptr[AW-1:0]];
    assign mem_q = q;
  end

  // --- crossing ------------------------------------------------------------
  if (ASYNC) begin : crossing
    // Gray code, registered on its own side, then two flip-flops on the
    // other. The register is the point: an exclusive-or of the binary
    // pointer's bits glitches through other codes as the bits of an
    // increment arrive at their own times, and a flip-flop on the other
    // clock can take one of those. Taken straight from the pointer, as it
    // first was here, the Husky Plus's fast FIFO overflowed and underflowed
    // on the part, more or less often from one placement to the next, and
    // no simulation could show it, having no glitches. A pointer a cycle
    // older only makes each side more cautious.
    reg [AW:0] wgray, rgray;
    always @(posedge wr_clk or posedge rst)
      if (rst) wgray <= 0;
      else wgray <= wptr ^ (wptr >> 1);
    always @(posedge rclk or posedge rst)
      if (rst) rgray <= 0;
      else rgray <= rptr ^ (rptr >> 1);
    (* ASYNC_REG = "TRUE" *) reg [AW:0] wgray_r1, wgray_r2;
    (* ASYNC_REG = "TRUE" *) reg [AW:0] rgray_w1, rgray_w2;
    always @(posedge rclk or posedge rst)
      if (rst) {wgray_r2, wgray_r1} <= 0;
      else {wgray_r2, wgray_r1} <= {wgray_r1, wgray};
    always @(posedge wr_clk or posedge rst)
      if (rst) {rgray_w2, rgray_w1} <= 0;
      else {rgray_w2, rgray_w1} <= {rgray_w1, rgray};
    // Converted back to binary in a register of its own. Unregistered,
    // the conversion's chain of exclusive-ors led straight into `fetch`
    // and the enable of every block RAM in the memory: on the Husky
    // Plus that was the latest path in `clk_usb`, at 18.75 ns of 10.4,
    // and one the FIFO Generator's built-in FIFO does not have. A
    // pointer a cycle older only makes each side more cautious.
    reg [AW:0] wbin_r, rbin_w;
    always @(posedge rclk or posedge rst)
      if (rst) wbin_r <= 0;
      else wbin_r <= gray_to_binary(wgray_r2);
    always @(posedge wr_clk or posedge rst)
      if (rst) rbin_w <= 0;
      else rbin_w <= gray_to_binary(rgray_w2);
    assign wptr_r = wbin_r;
    assign rptr_w = rbin_w;
    assign wptr_r_next = gray_to_binary(wgray_r2);
    assign rptr_w_next = gray_to_binary(rgray_w2);
  end else if (BANKED) begin : common_banked
    // The words in the memory: a banked memory is written a cycle after
    // the write is taken, so the write pointer a cycle late. With two
    // clocks the crossing is slower than that already.
    reg [AW:0] wptr_q;
    always @(posedge wr_clk or posedge rst)
      if (rst) wptr_q <= 0;
      else wptr_q <= wptr;
    assign wptr_r = wptr_q;
    assign rptr_w = rptr;
    assign wptr_r_next = wptr;
    assign rptr_w_next = rptr_next;
  end else begin : common
    assign wptr_r = wptr;
    assign rptr_w = rptr;
    assign wptr_r_next = wptr_next;
    assign rptr_w_next = rptr_next;
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

// 72 x 4096, independent clocks, fall-through, `prog_empty` at 6; on the
// Husky Plus 72 x 8192 (`cwhusky_plus.srcs`), holding 8,196 words here
// (`extra_words`).
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
`ifdef PLUS
      .AW(13),
      .EXTRA(3),
      // Banks, so segments, so no multiplexer of the mapping's own in front
      // of `stage`: see `husky_fifo`'s `banked`. Mapped as one memory it was
      // eight blocks deep, and on the part one of its four 18-bit columns
      // now and then gave a word other than the one read, which came back
      // as the halves of the two samples that straddle that column wrong.
      .BANK_AW(12),
`else
      .AW(12),
`endif
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
// `prog_empty` at 4. `usb_slow_fifo1.xci`; on the Husky Plus 72 x 16384
// and `prog_full` at 16390, which is past the depth, and holding 16,392
// words here (`extra_words`).
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
`ifdef PLUS
      .AW(14),
      .PROG_FULL(16390),
      .EXTRA(7),
`else
      .AW(11),
      .PROG_FULL(2047),
`endif
      .FWFT(1),
      .PROG_EMPTY(4),
      .BANK_AW(13)
  ) u_fifo (
      .rst(rst), .wr_clk(clk), .rd_clk(clk), .din(din), .wr_en(wr_en), .rd_en(rd_en),
      .dout(dout), .full(full), .empty(empty), .prog_full(prog_full),
      .prog_empty(prog_empty), .overflow(overflow), .underflow(underflow)
  );
endmodule

// 72 x 16384, common clock, fall-through, `prog_full` at 16384.
// `usb_slow_fifo2.xci`; on the Husky Plus 72 x 32768 and `prog_full` at
// 32782, holding 32,784 words here (`extra_words`).
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
`ifdef PLUS
      .AW(15),
      .PROG_FULL(32782),
      .EXTRA(15),
`else
      .AW(14),
      .PROG_FULL(16384),
`endif
      .FWFT(1),
      .BANK_AW(13)
  ) u_fifo (
      .rst(rst), .wr_clk(clk), .rd_clk(clk), .din(din), .wr_en(wr_en), .rd_en(rd_en),
      .dout(dout), .full(full), .empty(empty), .prog_full(prog_full),
      .prog_empty(), .overflow(overflow), .underflow(underflow)
  );
endmodule

// The Husky Plus's (`cwhusky_plus.xpr`). Its `.xci` is not in the
// checkout — the xpr reaches it through `../../plus_optimization/`, a
// path outside it — so these parameters are from NewAE's functional
// simulation netlist of the same core, `fpga/tracewhisperer/sim/vivado/
// fifo_generator_semipro_sim_netlist.v`: `C_DIN_WIDTH`, `C_WR_DEPTH`,
// `C_PRELOAD_REGS` 1 with `C_PRELOAD_LATENCY` 0 (fall-through),
// `C_PROG_FULL_THRESH_ASSERT_VAL`, `C_PROG_EMPTY_THRESH_ASSERT_VAL`,
// `C_HAS_OVERFLOW` 0 and `C_HAS_UNDERFLOW` 1.

// 18 x 16384, independent clocks, fall-through, `prog_full` at 14376
// and `prog_empty` at 128, no `overflow`. `fifo_generator_semipro`.
module fifo_generator_semipro (
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
      .AW(14),
      .FWFT(1),
      .ASYNC(1),
      .PROG_FULL(14376),
      .PROG_EMPTY(128)
  ) u_fifo (
      .rst(rst), .wr_clk(wr_clk), .rd_clk(rd_clk), .din(din), .wr_en(wr_en), .rd_en(rd_en),
      .dout(dout), .full(full), .empty(empty), .prog_full(prog_full),
      .prog_empty(prog_empty), .overflow(), .underflow(underflow)
  );
endmodule
