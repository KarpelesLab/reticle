// A construct the IR cannot express.
module error_unsupported(input logic clk, output logic [7:0] q);
    initial begin
        fork
            q = 8'd1;
            q = 8'd2;
        join
    end
endmodule
