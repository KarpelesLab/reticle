// A bit-select of a variable as the target of `force` and `release`: IEEE
// 1364-2005 §9.3.2 allows one of a vector net only, and xsim 2026.1 refuses
// it too (VRFC 10-3149). A bit-select of a net is forced on its own.
module error_force_part_var(input wire [3:0] a, output wire [3:0] y);
    reg [3:0] r;
    assign y = a;
    initial begin
        force r[1] = 1'b0;
        release r[1];
        force y[2] = 1'b1;
    end
endmodule
