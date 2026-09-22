// A four-way multiplexer written as a case statement.
module mux4(
    input  wire [1:0] sel,
    input  wire [7:0] a, b, c, d,
    output reg  [7:0] y
);
    always @* begin
        case (sel)
            2'd0: y = a;
            2'd1: y = b;
            2'd2: y = c;
            default: y = d;
        endcase
    end
endmodule
