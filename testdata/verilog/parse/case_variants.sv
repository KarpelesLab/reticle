// case, casez, casex, case inside, qualifiers, multiple patterns, default
// with and without colon, and nested cases.
module cases(input logic [3:0] op, input logic [7:0] v, output logic [7:0] y);
    always_comb begin
        case (op)
            4'd0: y = v;
            4'd1, 4'd2: y = v + 1;
            4'd3: begin
                y = v - 1;
            end
            default: y = 'x;
        endcase

        casez (op)
            4'b1???: y = 8'd1;
            4'b01??: y = 8'd2;
            4'b001?: y = 8'd3;
            default  y = 8'd0;
        endcase

        casex (op)
            4'b1xxx: y = 8'd1;
            4'bx1zz: y = 8'd2;
            default: ;
        endcase

        unique case (op) inside
            [4'd0:4'd3]: y = 8'd10;
            4'd4, [4'd6:4'd7]: y = 8'd11;
            default: y = 8'd12;
        endcase

        priority casez (op)
            4'b???1: y = 1;
            4'b??1?: y = 2;
        endcase

        unique0 case (1'b1)
            op[0]: y = 1;
            op[1]: y = 2;
        endcase

        case (v)
            8'h00: case (op)
                4'd0: y = 0;
                default: y = 1;
            endcase
            8'hff: y = 2;
        endcase

        case (op) endcase

        case (op == 4'd1 ? v : ~v)
            8'd5: y = 5;
        endcase
    end
endmodule
