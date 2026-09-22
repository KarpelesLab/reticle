// Attributes on modules, ports, declarations, instances, statements,
// case items and inside expressions, with and without values.
(* keep_hierarchy = "yes", top *)
module attrs #(
    parameter W = 4
) (
    (* keep *) input wire clk,
    (* mark_debug = 1, async_reg *) output reg [W-1:0] q,
    input wire [W-1:0] d
);
    (* ram_style = "block" *) reg [7:0] mem [0:255];
    (* dont_touch *) wire w = d[0];
    (* keep = "true" *) (* another *) reg r;

    (* fsm_encoding = "one-hot" *)
    always @(posedge clk) begin
        (* full_case, parallel_case *)
        case (d)
            0: q <= 1;
            default: q <= 0;
        endcase
        (* unroll *) for (r = 0; r < 1; r = r + 1) ;
        q <= (* precedence *) d;
        q <= d + (* op_attr *) 1;
    end

    (* keep_hierarchy *) leaf u0 (.i(d), .o(), .clk(clk));
    (* black_box *) function [3:0] f;
        input [3:0] x;
        f = x;
    endfunction
    (* async *) assign r = 1'b0;
endmodule
