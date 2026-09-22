// Parameter port lists, body parameters, type parameters, specparams and
// defparam.
module params #(
    parameter int W = 8,
    parameter type T = logic [W-1:0],
    localparam int W2 = W * 2,
    parameter real GAIN = 1.5,
    parameter string NAME = "params",
    parameter logic [3:0] MASK = 4'hf,
    INIT = 0,
    parameter signed [7:0] S = -1,
    parameter [7:0] R = 8'd3, R2 = 8'd4,
    parameter type U = int
) (
    input T in,
    output T out
);
    parameter P1 = 1, P2 = P1 + 1;
    localparam int L = W2 / 2;
    localparam type VT = logic [L-1:0];
    parameter [W-1:0] MEM_INIT [0:3] = '{0, 1, 2, 3};
    parameter bit [1:0] TWO = 2'b10;
    specparam TSETUP = 1.2, THOLD = 0.8;
    specparam TP = 1:2:3;
    VT vt;
    U u;
    assign out = T'(in & MASK);
endmodule

module empty_params #() (input a);
endmodule

module override;
    params #(.W(16), .T(logic [15:0]), .GAIN(2.0), .NAME("x"), .U(byte)) u0 (.in(), .out());
    params #(4, logic [3:0]) u1 (.in('0), .out());
    defparam u0.P1 = 5, u1.P1 = 6;
    defparam override.u0.P2 = 7;
endmodule
