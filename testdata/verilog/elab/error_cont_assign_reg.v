// A continuous assignment to a variable, which Verilog-2005 forbids.
module error_cont_assign_reg(input wire d, output reg q);
    assign q = d;
endmodule
