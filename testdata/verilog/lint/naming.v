// Names that do not follow the conventions the rule knows.
// lint-config: off:all warn:naming
module BadName #(parameter width = 4) (
    input  wire            CK,
    input  wire            rst,
    output reg [width-1:0] Q
);
    always @(posedge CK or negedge rst)
        Q <= {width{1'b0}};
endmodule
