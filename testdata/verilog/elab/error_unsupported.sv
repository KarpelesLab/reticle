// A construct the IR cannot express. It was `fork` / `join`, until the IR
// had processes started by a `fork`, then a queue, until a queue was a
// memory and its size; an associative array has nothing yet.
module error_unsupported(input logic clk, output logic [7:0] q);
    int pending [string];
    initial q = 8'd1;
endmodule
