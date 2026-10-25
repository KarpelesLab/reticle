// What `fifo_generator.v`'s stand-ins promise, checked in simulation:
//
//   * standard (`trigger_fifo`): words come back in order, each on `dout`
//     the cycle after its `rd_en`; `empty` is exact; a read of an empty
//     FIFO raises `underflow` the next cycle and changes nothing;
//   * fall-through (`usb_slow_fifo1`): the oldest word is on `dout`
//     whenever `empty` is low; `prog_empty` holds at four words and below
//     and `prog_full` at 2047 and above (16390 with `PLUS` defined);
//   * full (`adc_fast_fifo`, 2048 words): `full` after the last word, and a
//     write then raises `overflow` and is dropped;
//   * independent clocks (`fast_slower_fifo`): words written on one clock
//     come back in order on another, nine of every ten cycles apart;
//   * banked (`usb_slow_fifo2`, banks of 8192): a stream of 9000 words
//     written every cycle and read two cycles in three comes back in order
//     across the banks, the reads close behind the writes; then 16384
//     words (32768 with `PLUS`) make it `prog_full` and one more `full`,
//     the word on `dout` being out of the memory, and one more again
//     overflows; and all of them come back.
//
// It prints `PASS:` and finishes, or `FAIL:` and what differed. What it
// would not catch: the cycle a flag moves on after a write on the other
// clock, which is not the FIFO Generator's here and is not claimed to be.
`timescale 1ns / 1ps
module fifo_generator_tb;
`ifdef PLUS
  localparam PROG_FULL = 16390;
`else
  localparam PROG_FULL = 2047;
`endif
  reg clk = 0, rclk = 0, rst = 1;
  always #5 clk = ~clk;
  always #9 rclk = ~rclk;

  integer failures = 0;
  task check(input [255:0] what, input [71:0] got, input [71:0] want);
    if (got !== want) begin
      $display("FAIL: %0s: got %h, want %h", what, got, want);
      failures = failures + 1;
    end
  endtask

  // --- standard ------------------------------------------------------------
  reg [31:0] t_din = 0;
  reg t_wr = 0, t_rd = 0;
  wire [31:0] t_dout;
  wire t_full, t_empty, t_under;
  trigger_fifo u_t (
      .clk(clk), .rst(rst), .din(t_din), .wr_en(t_wr), .rd_en(t_rd), .dout(t_dout),
      .full(t_full), .empty(t_empty), .underflow(t_under)
  );

  // --- fall-through, thresholds ------------------------------------------
  reg [71:0] s_din = 0;
  reg s_wr = 0, s_rd = 0;
  wire [71:0] s_dout;
  wire s_full, s_empty, s_pe, s_pf, s_over, s_under;
  usb_slow_fifo1 u_s (
      .clk(clk), .rst(rst), .din(s_din), .wr_en(s_wr), .rd_en(s_rd), .dout(s_dout),
      .full(s_full), .empty(s_empty), .prog_empty(s_pe), .prog_full(s_pf),
      .overflow(s_over), .underflow(s_under)
  );

  // --- full and overflow --------------------------------------------------
  reg [11:0] a_din = 0;
  reg a_wr = 0, a_rd = 0;
  wire [11:0] a_dout;
  wire a_full, a_empty, a_over, a_under;
  adc_fast_fifo u_a (
      .clk(clk), .rst(rst), .din(a_din), .wr_en(a_wr), .rd_en(a_rd), .dout(a_dout),
      .full(a_full), .empty(a_empty), .overflow(a_over), .underflow(a_under)
  );

  // --- independent clocks -------------------------------------------------
  reg [71:0] x_din = 0;
  reg x_wr = 0, x_rd = 0;
  wire [71:0] x_dout;
  wire x_full, x_empty, x_pe, x_over, x_under;
  fast_slower_fifo u_x (
      .rst(rst), .wr_clk(clk), .rd_clk(rclk), .din(x_din), .wr_en(x_wr), .rd_en(x_rd),
      .dout(x_dout), .full(x_full), .empty(x_empty), .prog_empty(x_pe),
      .overflow(x_over), .underflow(x_under)
  );

  // --- banked ------------------------------------------------------------
  reg [71:0] b_din = 0;
  reg b_wr = 0, b_rd = 0;
  wire [71:0] b_dout;
  wire b_full, b_empty, b_pf, b_over, b_under;
  usb_slow_fifo2 u_b (
      .clk(clk), .rst(rst), .din(b_din), .wr_en(b_wr), .rd_en(b_rd), .dout(b_dout),
      .full(b_full), .empty(b_empty), .prog_full(b_pf), .overflow(b_over),
      .underflow(b_under)
  );
  // Its depth: on the Husky Plus 32768, and `prog_full` past what it holds.
`ifdef PLUS
  localparam B_DEPTH = 32768;
`else
  localparam B_DEPTH = 16384;
`endif
  reg b_go = 0;
  integer b_sent = 0, b_got = 0, b_cycle = 0, b_wrong = 0;
  always @(posedge clk)
    if (b_go && b_sent < 9000) begin
      b_din <= 72'hbe_0000_0000_0000_0000 + b_sent;
      b_wr <= 1;
      b_sent <= b_sent + 1;
    end else if (b_go) begin
      b_wr <= 0;
    end
  always @(negedge clk)
    if (b_go) begin
      b_cycle = b_cycle + 1;
      if (!b_empty && b_got < 9000 && b_cycle % 3 != 0) begin
        if (b_dout !== 72'hbe_0000_0000_0000_0000 + b_got) begin
          if (b_wrong == 0) $display("FAIL: banked word %0d: got %h", b_got, b_dout);
          b_wrong = b_wrong + 1;
        end
        b_rd = 1;
        b_got = b_got + 1;
      end else begin
        b_rd = 0;
      end
    end

  integer i;
  integer got = 0;
  integer sent = 0;
  reg go = 0;
  // The crossing's writer and reader, each on its own clock.
  always @(posedge clk)
    if (go && sent < 40) begin
      x_din <= 72'hcd_0000_0000_0000_0000 + sent;
      x_wr <= 1;
      sent <= sent + 1;
    end else begin
      x_wr <= 0;
    end
  // Sampled between edges, so `rd_en` spans the edge that pops.
  always @(negedge rclk)
    if (go && !x_empty && got < 40) begin
      check("crossed word", x_dout, 72'hcd_0000_0000_0000_0000 + got);
      x_rd = 1;
      got = got + 1;
    end else begin
      x_rd = 0;
    end

  initial begin
    repeat (4) @(posedge clk);
    rst <= 0;
    repeat (4) @(posedge rclk);
    @(posedge clk);

    // Standard: five words in, five out, in order, a cycle after rd_en.
    check("standard starts empty", t_empty, 1);
    for (i = 0; i < 5; i = i + 1) begin
      t_din <= 32'h1000 + i;
      t_wr <= 1;
      @(posedge clk);
    end
    t_wr <= 0;
    @(posedge clk);
    check("standard not empty", t_empty, 0);
    for (i = 0; i < 5; i = i + 1) begin
      t_rd <= 1;
      @(posedge clk);
      t_rd <= 0;
      @(posedge clk);
      check("standard word", t_dout, 32'h1000 + i);
    end
    check("standard empty again", t_empty, 1);
    t_rd <= 1;
    @(posedge clk);
    t_rd <= 0;
    @(posedge clk);
    check("standard underflow", t_under, 1);
    check("standard word kept", t_dout, 32'h1004);
    @(posedge clk);
    check("standard underflow is one cycle", t_under, 0);

    // Fall-through: the oldest word waits on dout; prog_empty at <= 4.
    for (i = 0; i < 4; i = i + 1) begin
      s_din <= 72'hab_0000_0000_0000_0000 + i;
      s_wr <= 1;
      @(posedge clk);
    end
    s_wr <= 0;
    repeat (3) @(posedge clk);
    check("fall-through has a word", s_empty, 0);
    check("fall-through oldest word", s_dout, 72'hab_0000_0000_0000_0000);
    check("prog_empty at four", s_pe, 1);
    s_din <= 72'hab_0000_0000_0000_0004;
    s_wr <= 1;
    @(posedge clk);
    s_wr <= 0;
    repeat (2) @(posedge clk);
    check("prog_empty above four", s_pe, 0);
    for (i = 0; i < 5; i = i + 1) begin
      check("fall-through word", s_dout, 72'hab_0000_0000_0000_0000 + i);
      s_rd <= 1;
      @(posedge clk);
      s_rd <= 0;
      @(posedge clk);
    end
    check("fall-through empty", s_empty, 1);
    // prog_full: 2047 words, or the Husky Plus's 16390.
    for (i = 0; i < PROG_FULL - 1; i = i + 1) begin
      s_wr <= 1;
      @(posedge clk);
    end
    s_wr <= 0;
    @(posedge clk);
    check("prog_full below its threshold", s_pf, 0);
    s_wr <= 1;
    @(posedge clk);
    s_wr <= 0;
    @(posedge clk);
`ifdef PLUS
    // The Husky Plus's threshold, 16390, is past the 16385 words this
    // holds, so the flag never rises; whether the core's does is not
    // known here.
    check("prog_full past the depth", s_pf, 0);
`else
    check("prog_full at its threshold", s_pf, 1);
`endif

    // Full and overflow: 2048 words, then one more is refused.
    for (i = 0; i < 2048; i = i + 1) begin
      a_din <= i;
      a_wr <= 1;
      @(posedge clk);
    end
    a_wr <= 0;
    @(posedge clk);
    check("full", a_full, 1);
    a_din <= 12'hfff;
    a_wr <= 1;
    @(posedge clk);
    a_wr <= 0;
    @(posedge clk);
    check("overflow", a_over, 1);
    a_rd <= 1;
    @(posedge clk);
    a_rd <= 0;
    @(posedge clk);
    check("first word after overflow", a_dout, 12'h000);
    check("not full after a read", a_full, 0);

    // Independent clocks: forty words across, in order.
    go = 1;
    i = 0;
    while (got < 40 && i < 2000) begin
      @(posedge clk);
      i = i + 1;
    end
    check("crossed all forty", got, 40);
    check("crossed underflow never", x_under, 0);
    check("crossed overflow never", x_over, 0);

    // Banked: the stream, then full.
    b_go = 1;
    i = 0;
    while (b_got < 9000 && i < 20000) begin
      @(posedge clk);
      i = i + 1;
    end
    // Stopped between edges, so that the last read's `rd_en` spans its
    // edge: stopped on one, it raced the FIFO for it.
    @(negedge clk);
    b_go = 0;
    b_rd = 0;
    @(posedge clk);
    check("banked streamed all", b_got, 9000);
    check("banked words in order", b_wrong, 0);
    check("banked underflow never", b_under, 0);
    check("banked empty after", b_empty, 1);
    for (i = 0; i < B_DEPTH; i = i + 1) begin
      b_din <= i;
      b_wr <= 1;
      @(posedge clk);
    end
    b_wr <= 0;
    @(posedge clk);
    // The word on `dout` is out of the memory, so it takes one more.
`ifndef PLUS
    check("banked prog_full at its depth", b_pf, 1);
`endif
    check("banked not full at its depth", b_full, 0);
    b_din <= B_DEPTH;
    b_wr <= 1;
    @(posedge clk);
    b_wr <= 0;
    @(posedge clk);
    check("banked full at one more", b_full, 1);
    check("banked oldest word when full", b_dout, 72'h0);
    b_wr <= 1;
    @(posedge clk);
    b_wr <= 0;
    @(posedge clk);
    check("banked overflow", b_over, 1);
    // And every word comes back, so that no bank's write landed in another.
    // Sampled between edges, so `rd_en` spans the edge that pops.
    b_wrong = 0;
    for (i = 0; i <= B_DEPTH; i = i + 1) begin
      @(negedge clk);
      if (b_dout !== i) begin
        if (b_wrong == 0) $display("FAIL: banked word %0d of the fill: got %h", i, b_dout);
        b_wrong = b_wrong + 1;
      end
      b_rd = 1;
    end
    @(negedge clk);
    b_rd = 0;
    @(posedge clk);
    check("banked fill read back", b_wrong, 0);
    check("banked empty after the fill", b_empty, 1);

    if (failures == 0) $display("PASS: the FIFO stand-ins");
    else $display("FAIL: %0d check(s)", failures);
    $finish;
  end
endmodule
