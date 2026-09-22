// An override of a parameter the module does not declare.
module epo_leaf #(parameter W = 4) (input wire [W-1:0] a, output wire [W-1:0] y);
    assign y = a;
endmodule

module error_param_override(input wire [3:0] a, output wire [3:0] y);
    epo_leaf #(.DEPTH(8)) u0 (.a(a), .y(y));
endmodule
