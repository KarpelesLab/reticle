// `casex`, and `x` where it can never match.
// lint-config: off:all warn:case-x-z
module cxz (input wire [3:0] op, output reg y);
    always @* begin
        casex (op)
            4'b1xxx: y = 1'b1;
            default: y = 1'b0;
        endcase

        case (op)
            4'b1x0z: y = 1'b1;
            default: y = 1'b0;
        endcase

        casez (op)
            4'b1?x?: y = 1'b1;
            default: y = 1'b0;
        endcase
    end
endmodule
