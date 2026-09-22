// A generate loop unrolled into hierarchical names, with a nested
// generate if.
module genfor #(parameter N = 4) (
    input  wire [N-1:0] a,
    input  wire [N-1:0] b,
    output wire [N-1:0] y
);
    genvar i;
    generate
        for (i = 0; i < N; i = i + 1) begin : g_bit
            wire t;
            assign t = a[i] & b[i];
            if (i == 0) begin : g_first
                assign y[i] = t;
            end else begin : g_rest
                assign y[i] = t | a[i-1];
            end
        end
    endgenerate
endmodule
