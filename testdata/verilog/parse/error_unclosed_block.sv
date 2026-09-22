// An unclosed begin/end and a missing endcase: the enclosing construct
// reports the wrong end keyword once, and the next module is intact.
module broken(input logic clk, input logic [1:0] s, output logic y);
    always_ff @(posedge clk) begin
        y <= s[0];
        if (s[1]) begin
            y <= 1'b0;
    end

    always_comb begin
        case (s)
            2'd0: y = 1'b1;
            2'd1: y = 1'b0;
        y = 1'b1;
    end
endmodule

module fine(input a, output b);
    assign b = a;
endmodule
