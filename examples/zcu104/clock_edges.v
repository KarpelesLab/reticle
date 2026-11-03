// Which edge of PL_CLK0 each clock region's flip-flops load on, read
// over EMIO with no AXI slave (docs/fpga-uray.md, "One clock region of
// the four runs on the other edge").
//
// A toggle t in the region of rows 165-180 is sampled by one flip-flop in
// each region (placed by clock_edges.rcf), and each sample is registered
// again and compared with t in t's region. A sampler that loads on the
// same edge as t's region reads t one clock late, so after one more
// register it equals t and sets eq; one on the other edge reads it half a
// clock late and sets ne. The flags arm after 16 clocks.
//
// EMIO_I(2k) is ne and EMIO_I(2k+1) is eq of sampler k: 0 c0, 1 c1n,
// 2 d0, 3 e0, 4 e1n, 5 e2l, 6 e3h, 7 e4e, 8 g0. Names ending in n sample
// on the falling edge. Read at 25 MHz, the right answer is 0x2a9a6:
// every rising-edge sampler eq, the two falling-edge ones ne.
//
//   reticle fpga examples/zcu104/clock_edges.v --device xczu7ev-ffvc1156 \
//       --constraints examples/zcu104/clock_edges.rcf --east 36 \
//       --bitstream clock_edges.bit --borrow-clock base.bit
module clock_edges (input wire clk, output wire [17:0] f);
  (* keep *) reg t = 1'b0;
  always @(posedge clk) t <= ~t;
  // The flags arm only once the pipeline has filled.
  (* keep *) reg [4:0] tw = 5'd0;
  always @(posedge clk) if (!tw[4]) tw <= tw + 5'd1;
  (* keep *) reg x_c0 = 1'b0;
  always @(posedge clk) x_c0 <= t;
  (* keep *) reg q_c0 = 1'b0;
  always @(posedge clk) q_c0 <= x_c0;
  (* keep *) reg eq_c0 = 1'b0, ne_c0 = 1'b0;
  always @(posedge clk) if (tw[4]) begin eq_c0 <= eq_c0 | (q_c0 == t); ne_c0 <= ne_c0 | (q_c0 != t); end
  (* keep *) reg x_c1n = 1'b0;
  always @(negedge clk) x_c1n <= t;
  (* keep *) reg q_c1n = 1'b0;
  always @(posedge clk) q_c1n <= x_c1n;
  (* keep *) reg eq_c1n = 1'b0, ne_c1n = 1'b0;
  always @(posedge clk) if (tw[4]) begin eq_c1n <= eq_c1n | (q_c1n == t); ne_c1n <= ne_c1n | (q_c1n != t); end
  (* keep *) reg x_d0 = 1'b0;
  always @(posedge clk) x_d0 <= t;
  (* keep *) reg q_d0 = 1'b0;
  always @(posedge clk) q_d0 <= x_d0;
  (* keep *) reg eq_d0 = 1'b0, ne_d0 = 1'b0;
  always @(posedge clk) if (tw[4]) begin eq_d0 <= eq_d0 | (q_d0 == t); ne_d0 <= ne_d0 | (q_d0 != t); end
  (* keep *) reg x_e0 = 1'b0;
  always @(posedge clk) x_e0 <= t;
  (* keep *) reg q_e0 = 1'b0;
  always @(posedge clk) q_e0 <= x_e0;
  (* keep *) reg eq_e0 = 1'b0, ne_e0 = 1'b0;
  always @(posedge clk) if (tw[4]) begin eq_e0 <= eq_e0 | (q_e0 == t); ne_e0 <= ne_e0 | (q_e0 != t); end
  (* keep *) reg x_e1n = 1'b0;
  always @(negedge clk) x_e1n <= t;
  (* keep *) reg q_e1n = 1'b0;
  always @(posedge clk) q_e1n <= x_e1n;
  (* keep *) reg eq_e1n = 1'b0, ne_e1n = 1'b0;
  always @(posedge clk) if (tw[4]) begin eq_e1n <= eq_e1n | (q_e1n == t); ne_e1n <= ne_e1n | (q_e1n != t); end
  (* keep *) reg x_e2l = 1'b0;
  always @(posedge clk) x_e2l <= t;
  (* keep *) reg q_e2l = 1'b0;
  always @(posedge clk) q_e2l <= x_e2l;
  (* keep *) reg eq_e2l = 1'b0, ne_e2l = 1'b0;
  always @(posedge clk) if (tw[4]) begin eq_e2l <= eq_e2l | (q_e2l == t); ne_e2l <= ne_e2l | (q_e2l != t); end
  (* keep *) reg x_e3h = 1'b0;
  always @(posedge clk) x_e3h <= t;
  (* keep *) reg q_e3h = 1'b0;
  always @(posedge clk) q_e3h <= x_e3h;
  (* keep *) reg eq_e3h = 1'b0, ne_e3h = 1'b0;
  always @(posedge clk) if (tw[4]) begin eq_e3h <= eq_e3h | (q_e3h == t); ne_e3h <= ne_e3h | (q_e3h != t); end
  (* keep *) reg x_e4e = 1'b0;
  always @(posedge clk) x_e4e <= t;
  (* keep *) reg q_e4e = 1'b0;
  always @(posedge clk) q_e4e <= x_e4e;
  (* keep *) reg eq_e4e = 1'b0, ne_e4e = 1'b0;
  always @(posedge clk) if (tw[4]) begin eq_e4e <= eq_e4e | (q_e4e == t); ne_e4e <= ne_e4e | (q_e4e != t); end
  (* keep *) reg x_g0 = 1'b0;
  always @(posedge clk) x_g0 <= t;
  (* keep *) reg q_g0 = 1'b0;
  always @(posedge clk) q_g0 <= x_g0;
  (* keep *) reg eq_g0 = 1'b0, ne_g0 = 1'b0;
  always @(posedge clk) if (tw[4]) begin eq_g0 <= eq_g0 | (q_g0 == t); ne_g0 <= ne_g0 | (q_g0 != t); end
  assign f = {eq_g0, ne_g0, eq_e4e, ne_e4e, eq_e3h, ne_e3h, eq_e2l, ne_e2l, eq_e1n, ne_e1n, eq_e0, ne_e0, eq_d0, ne_d0, eq_c1n, ne_c1n, eq_c0, ne_c0};
endmodule
