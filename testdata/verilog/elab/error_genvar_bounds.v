// A generate loop whose bound is not constant.
module error_genvar_bounds(input wire [3:0] n, output wire [3:0] y);
    genvar i;
    generate
        for (i = 0; i < n; i = i + 1) begin : g
            assign y[i] = 1'b0;
        end
    endgenerate
endmodule
