// Needs NewAE's `fifo_sync.v`, `fifo_async.v` and `async_resetFFstyle2.v`
// from a chipwhisperer-husky-fpga checkout (`fpga/fpga-common/hdl/`):
//
//   reticle sim xpm_memory_sdpram_tb.v xpm_memory_sdpram.v \
//       husky/fpga/fpga-common/hdl/{fifo_sync,fifo_async,async_resetFFstyle2}.v
//
// The `xpm_memory_sdpram` stand-in, checked two ways.
//
// 1. Against the macro's own description (UG974), as a shadow memory
//    this bench keeps for itself: random writes, reads, read enables and
//    output resets, with one clock and with two. On each read edge
//    `doutb` must be what the shadow held at that address before any
//    write on the same edge ("read_first"), zero after `rstb`, and
//    unchanged with `enb` low.
// 2. Under NewAE's own FIFOs, in standard (not fall-through) mode:
//    `fifo_sync` and `fifo_async` built twice from the same stimulus,
//    once on block RAM through the stand-in (`pBRAM`) and once on the
//    flip-flop memory NewAE simulates with (`pFLOPS`). Data, empty, full,
//    the thresholds, overflow and underflow must agree on every edge.
//
// Fall-through mode is not compared, because there NewAE's two memories
// differ by design, and would differ on Xilinx's macro too: the block
// RAM path enables its read with `~empty`, so on the edge `empty` falls
// the first word has not been read yet and `rdata` holds the last one;
// and it reads ahead at the next address, which on the edge that address
// is written returns the old word (the source warns of it under
// `__ICARUS__`). Read off `fifo_sync.v` and `fifo_async.v`, and found by
// an earlier version of this bench disagreeing. NewAE's regressions run
// the Husky with these FIFOs on Vivado's own model (`sim/makefile`,
// `FIFO_TYPE=NOXILINXFIFO`), so their logic around the FIFOs copes with
// both; that is quoted, not checked.
//
// Writes are offered only to a FIFO that is not full: NewAE's block RAM
// path gives the memory the raw `wen`, so a write to a full FIFO
// overwrites its oldest word there and not in the flip-flop memory.
//
// Would catch a read latency other than one, the new word read where the
// old is due, `rstb` ignored or not taking priority, `enb` ignored, or
// the wrong clock reading; would not catch the shadow sharing a mistaken
// reading of UG974 with the stand-in, nor anything about block RAM on
// the part, since this is the behavioural model in simulation.
`timescale 1ns / 1ps
module xpm_memory_sdpram_tb;
  reg clk = 0, rclk = 0;
  always #5 clk = ~clk;
  always #7 rclk = ~rclk;
  integer seed = 7, cycles = 0;
  // Starts high and falls: NewAE's FIFOs synchronise the reset with an
  // asynchronous-reset flip-flop, which needs an edge.
  reg rst_n = 1;
  reg checking = 0;

  // --- 1: the macro on its own ---------------------------------------
  reg [4:0] addra = 0, addrb = 0;
  reg [11:0] dina = 0;
  reg wea = 0, enb = 0, rstb = 0;
  wire [11:0] out_common, out_two;
  xpm_memory_sdpram #(
      .ADDR_WIDTH_A(5), .ADDR_WIDTH_B(5), .BYTE_WRITE_WIDTH_A(12),
      .CLOCKING_MODE("common_clock"), .MEMORY_PRIMITIVE("block"), .MEMORY_SIZE(384),
      .READ_DATA_WIDTH_B(12), .READ_LATENCY_B(1), .READ_RESET_VALUE_B("0"),
      .WRITE_DATA_WIDTH_A(12), .WRITE_MODE_B("read_first")
  ) common (
      .dbiterrb(), .doutb(out_common), .sbiterrb(), .addra(addra), .addrb(addrb),
      .clka(clk), .clkb(1'b0), .dina(dina), .ena(1'b1), .enb(enb),
      .injectdbiterra(1'b0), .injectsbiterra(1'b0), .regceb(1'b1), .rstb(rstb),
      .sleep(1'b0), .wea(wea)
  );
  // Two clocks: the read port on `rclk`, its controls from that domain.
  reg [4:0] addrb2 = 0;
  reg enb2 = 0, rstb2 = 0;
  xpm_memory_sdpram #(
      .ADDR_WIDTH_A(5), .ADDR_WIDTH_B(5), .BYTE_WRITE_WIDTH_A(12),
      .CLOCKING_MODE("independent_clock"), .MEMORY_PRIMITIVE("block"), .MEMORY_SIZE(384),
      .READ_DATA_WIDTH_B(12), .READ_LATENCY_B(1), .READ_RESET_VALUE_B("0"),
      .WRITE_DATA_WIDTH_A(12), .WRITE_MODE_B("read_first")
  ) two (
      .dbiterrb(), .doutb(out_two), .sbiterrb(), .addra(addra), .addrb(addrb2),
      .clka(clk), .clkb(rclk), .dina(dina), .ena(1'b1), .enb(enb2),
      .injectdbiterra(1'b0), .injectsbiterra(1'b0), .regceb(1'b1), .rstb(rstb2),
      .sleep(1'b0), .wea(wea)
  );

  // The shadow and what each output must be, worked out on the edge
  // before the shadow takes that edge's write.
  reg [11:0] shadow [0:31];
  reg [11:0] want_common = 0, want_two = 0;
  integer i;
  initial for (i = 0; i < 32; i = i + 1) shadow[i] = 0;
  always @(posedge clk) begin
    if (rstb) want_common <= 0;
    else if (enb) want_common <= shadow[addrb];
    if (wea) shadow[addra] <= dina;
  end
  always @(posedge rclk) begin
    if (rstb2) want_two <= 0;
    else if (enb2) want_two <= shadow[addrb2];
  end
  // Every word written once before the reads count, so that a read of
  // an unwritten word does not compare two unknowns.
  always @(posedge clk) begin
    if (cycles < 40) begin
      addra <= cycles; wea <= 1; dina <= $random(seed);
    end else begin
      addra <= $random(seed); wea <= $random(seed); dina <= $random(seed);
    end
    addrb <= $random(seed); enb <= $random(seed); rstb <= ($random(seed) & 7) == 0;
  end
  always @(posedge rclk) begin
    addrb2 <= $random(seed); enb2 <= $random(seed); rstb2 <= ($random(seed) & 7) == 0;
  end

  // --- 2: NewAE's FIFOs, standard mode -------------------------------
  reg want_w = 0, ren = 0, want_aw = 0, aren = 0;
  wire wen, awen;
  reg [11:0] wdata = 0, awdata = 0;
  wire [23:0] d, ad;  // flat: [11:0] block RAM, [23:12] flip-flops
  wire [1:0] full, empty, full_t, empty_t, over, under;
  wire [1:0] afull, aempty, afull_t, aempty_t, aover, aunder;
  genvar g;
  for (g = 0; g < 2; g = g + 1) begin : fifos
    fifo_sync #(
        .pDATA_WIDTH(12), .pDEPTH(32), .pFALLTHROUGH(0),
        .pFLOPS(g), .pDISTRIBUTED(0), .pBRAM(1 - g)
    ) s (
        .clk(clk), .rst_n(rst_n),
        .full_threshold_value(32'd28), .empty_threshold_value(32'd3),
        .wen(wen), .wdata(wdata), .full(full[g]), .almost_full(),
        .overflow(over[g]), .full_threshold(full_t[g]),
        .ren(ren), .rdata(d[g*12 +: 12]), .empty(empty[g]), .almost_empty(),
        .empty_threshold(empty_t[g]), .underflow(under[g])
    );
    fifo_async #(
        .pDATA_WIDTH(12), .pDEPTH(32), .pFALLTHROUGH(0),
        .pFLOPS(g), .pDISTRIBUTED(0), .pBRAM(1 - g)
    ) a (
        .wclk(clk), .rclk(rclk), .wrst_n(rst_n), .rrst_n(rst_n),
        .wfull_threshold_value(32'd28), .rempty_threshold_value(32'd3),
        .wen(awen), .wdata(awdata), .wfull(afull[g]), .walmost_full(),
        .woverflow(aover[g]), .wfull_threshold(afull_t[g]),
        .ren(aren), .rdata(ad[g*12 +: 12]), .rempty(aempty[g]), .ralmost_empty(),
        .rempty_threshold(aempty_t[g]), .runderflow(aunder[g])
    );
  end
  // A write is offered only when the FIFO is not full, combinationally:
  // an enable registered on last cycle's flag still writes on the cycle
  // `full` rises.
  assign wen = want_w && !full[0];
  assign awen = want_aw && !afull[0];
  // Phases of mostly writing and mostly reading, so both ends are hit;
  // reads of an empty FIFO are offered too, for `underflow`.
  integer phase = 0;
  always @(posedge clk) begin
    cycles <= cycles + 1;
    if (cycles % 150 == 0) phase <= $random(seed) & 1;
    want_w <= rst_n && (($random(seed) & 3) != 0) == (phase == 0);
    ren <= rst_n && (($random(seed) & 3) != 0) == (phase != 0);
    wdata <= $random(seed);
    want_aw <= rst_n && (($random(seed) & 3) != 0) == (phase == 0);
    awdata <= $random(seed);
  end
  always @(posedge rclk) aren <= rst_n && (($random(seed) & 3) != 0) == (phase != 0);
  // Standard data is due the cycle after a read that was taken.
  reg took = 0, atook = 0;
  always @(posedge clk) took <= ren && !empty[0];
  always @(posedge rclk) atook <= aren && !aempty[0];

  // --- comparisons, on the falling edges -----------------------------
  // One count per clock's comparisons: a variable has one driver.
  integer errors_w = 0, errors_r = 0, unknown = 0;
  integer filled = 0, drained = 0, underflows = 0, reads_common = 0, reads_two = 0;
  function bad(input [8*8-1:0] what, input [31:0] got, input [31:0] want);
    begin
      bad = got !== want;
      if (bad) $display("FAIL: %0s at %0t: %h, wanted %h", what, $time, got, want);
    end
  endfunction
  always @(negedge clk) if (checking) begin
    errors_w = errors_w + bad("common", out_common, want_common);
    errors_w = errors_w + bad("sync em", empty[0], empty[1]);
    errors_w = errors_w + bad("sync fu", full[0], full[1]);
    errors_w = errors_w + bad("sync ov", over[0], over[1]);
    errors_w = errors_w + bad("sync un", under[0], under[1]);
    errors_w = errors_w + bad("sync ft", full_t[0], full_t[1]);
    errors_w = errors_w + bad("sync et", empty_t[0], empty_t[1]);
    errors_w = errors_w + bad("async fu", afull[0], afull[1]);
    errors_w = errors_w + bad("async ov", aover[0], aover[1]);
    errors_w = errors_w + bad("async ft", afull_t[0], afull_t[1]);
    if (took) errors_w = errors_w + bad("sync d", d[11:0], d[23:12]);
    // Two undefined flags compare equal under `!==`; a bench that let
    // them would pass having checked nothing.
    if (^{full, empty, over, under, full_t, empty_t, afull, aover, afull_t} === 1'bx)
      unknown = unknown + 1;
    if (full[0]) filled = filled + 1;
    if (empty[0]) drained = drained + 1;
    if (under[0]) underflows = underflows + 1;
    if (enb && !rstb) reads_common = reads_common + 1;
  end
  integer aunknown = 0;
  always @(negedge rclk) if (checking) begin
    errors_r = errors_r + bad("two", out_two, want_two);
    errors_r = errors_r + bad("async em", aempty[0], aempty[1]);
    errors_r = errors_r + bad("async un", aunder[0], aunder[1]);
    errors_r = errors_r + bad("async et", aempty_t[0], aempty_t[1]);
    if (atook) errors_r = errors_r + bad("async d", ad[11:0], ad[23:12]);
    if (^{aempty, aunder, aempty_t} === 1'bx) aunknown = aunknown + 1;
    if (enb2 && !rstb2) reads_two = reads_two + 1;
  end

  initial begin
    #3 rst_n = 0;
    repeat (5) @(posedge clk);
    rst_n = 1;
    repeat (45) @(posedge clk);
    checking = 1;
    repeat (6000) @(posedge clk);
    if (unknown + aunknown > 0)
      $display("FAIL: flags undefined on %0d and %0d edges", unknown, aunknown);
    else if (filled < 20 || drained < 20 || underflows < 20 || reads_common < 1000 || reads_two < 1000)
      $display("FAIL: the stimulus filled the FIFO on %0d cycles, drained it on %0d, underflowed %0d, read %0d and %0d",
               filled, drained, underflows, reads_common, reads_two);
    else if (errors_w + errors_r == 0)
      $display("PASS: the xpm_memory_sdpram stand-in reads as UG974 describes, and NewAE's FIFOs on it agree with their flip-flop memories");
    else $display("FAIL: %0d disagreement(s)", errors_w + errors_r);
    $finish;
  end
endmodule
