// A case that leaves values out, and paths that do not assign.
// lint-config: off:all warn:incomplete-case warn:latch-inferred
module decode (
    input  logic [1:0] sel,
    input  logic       en,
    output logic [3:0] y,
    output logic       hit,
    output logic [3:0] guarded
);
    always_comb begin
        case (sel)               // 2 of 4 values, no default
            2'd0: y = 4'b0001;
            2'd1: y = 4'b0010;
        endcase
    end

    always_comb if (en) hit = 1'b1;   // no `else`: a latch

    always_comb begin                 // a default value first: no latch
        guarded = 4'b0000;
        if (en) guarded = 4'b1111;
    end
endmodule
