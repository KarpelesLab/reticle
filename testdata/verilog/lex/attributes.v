(* keep_hierarchy = "yes" *)
module attrs (
    (* keep *) input wire clk,
    (* mark_debug = 1, async_reg *) output reg q
);
    (* ram_style = "block" *) reg [7:0] mem [0:255];

    always @(*) q = clk;
    always @( * ) q = clk;
    always @* q = clk;
    always @(clk) q = (* full_case *) clk;

    // Not an attribute: multiplication then a close paren.
    wire w = (a * b);
    wire v = (a*b*)c;
endmodule
