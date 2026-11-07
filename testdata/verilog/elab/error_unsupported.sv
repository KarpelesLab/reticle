// A construct the IR cannot express. It was `fork` / `join`, until the IR
// had processes started by a `fork`; a queue has nothing yet.
module error_unsupported(input logic clk, output logic [7:0] q);
    int pending [$];
    initial q = 8'd1;
endmodule
