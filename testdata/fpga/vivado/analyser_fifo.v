// The ChipWhisperer-Husky Plus analyser's FIFO, `fifo_generator_semipro`
// (the stand-in in examples/husky/fifo_generator.v), alone: written at the
// analyser's 400 MHz and read at the USB interface's 96 MHz. The data is
// shifted in a bit at a time and the word read is folded to one bit, so
// that the design needs eight pins; every port is registered, so each path
// is the FIFO's own.
module analyser_fifo (
    input  wire wr_clk,
    input  wire rd_clk,
    input  wire din,
    input  wire wr_en,
    input  wire rd_en,
    output reg  dout,
    output reg  full,
    output reg  empty
);
  reg [17:0] wdata;
  reg        we, re;
  wire [17:0] q;
  wire f, e, pf, pe, under;
  always @(posedge wr_clk) begin
    wdata <= {wdata[16:0], din};
    we <= wr_en;
    full <= f | pf;
  end
  always @(posedge rd_clk) begin
    re <= rd_en;
    dout <= ^q;
    empty <= e | pe | under;
  end
  fifo_generator_semipro u_fifo (
      .rst(1'b0), .wr_clk(wr_clk), .rd_clk(rd_clk), .din(wdata), .wr_en(we), .rd_en(re),
      .dout(q), .full(f), .empty(e), .prog_full(pf), .prog_empty(pe), .underflow(under)
  );
endmodule
