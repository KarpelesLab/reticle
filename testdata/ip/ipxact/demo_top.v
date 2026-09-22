// The one file the user of the imported package writes: a top level
// that instantiates the counter `simple.xml` describes.
module demo_top(
  input        clk,
  input        rst_n,
  input        clear,
  output [7:0] count
);
  counter u_counter (
    .clk   (clk),
    .rst_n (rst_n),
    .clear (clear),
    .count (count)
  );
endmodule
