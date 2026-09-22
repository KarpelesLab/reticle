// $clog2, $bits and parameter arithmetic in dimensions.
module clog2_params #(
    parameter int DEPTH = 100,
    parameter int WIDTH = 8
) (
    input  logic                     clk,
    input  logic [$clog2(DEPTH)-1:0] addr,
    output logic [31:0]              addr_bits,
    output logic [WIDTH-1:0]         q
);
    localparam int AW = $clog2(DEPTH);
    logic [WIDTH-1:0] store [DEPTH];

    always_ff @(posedge clk)
        q <= store[addr];

    assign addr_bits = $bits(addr);
endmodule
