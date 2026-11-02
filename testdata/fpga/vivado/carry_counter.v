// A 32-bit counter with a terminal compare, the shape of NewAE's
// `reg_la.capture_count`, the analyser's sample counter at 400 MHz. The
// depth it compares with is shifted in a bit at a time so that the design
// needs four pins.
module carry_counter (
    input  wire clk,
    input  wire go,
    input  wire din,
    output reg  done
);
  reg [31:0] depth;
  reg [31:0] count;
  always @(posedge clk) begin
    depth <= {depth[30:0], din};
    if (go) count <= 0;
    else count <= count + 1;
    done <= count == depth;
  end
endmodule
