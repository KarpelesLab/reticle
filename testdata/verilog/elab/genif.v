// Generate if and generate case selecting an implementation.
module genif #(parameter MODE = 1) (
    input  wire [3:0] a,
    output wire [3:0] y,
    output wire [3:0] z
);
    generate
        if (MODE == 0) begin : g_pass
            assign y = a;
        end else begin : g_invert
            assign y = ~a;
        end
    endgenerate

    generate
        case (MODE)
            0: begin : g_zero
                assign z = 4'd0;
            end
            1, 2: begin : g_one
                assign z = a + 4'd1;
            end
            default: begin : g_x
                assign z = 4'd15;
            end
        endcase
    endgenerate
endmodule
