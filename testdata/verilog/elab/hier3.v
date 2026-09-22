// A three-level hierarchy: top instantiates mid, mid instantiates leaf.
module leaf #(parameter W = 4) (
    input  wire [W-1:0] a,
    output wire [W-1:0] y
);
    assign y = ~a;
endmodule

module mid #(parameter W = 4) (
    input  wire [W-1:0] a,
    output wire [W-1:0] y
);
    wire [W-1:0] t;
    leaf #(.W(W)) u_leaf (.a(a), .y(t));
    assign y = t ^ {W{1'b1}};
endmodule

module hier_top(
    input  wire [7:0] a,
    output wire [7:0] y
);
    mid #(.W(8)) u_mid (.a(a), .y(y));
endmodule
