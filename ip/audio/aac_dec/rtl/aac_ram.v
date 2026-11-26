// aac_ram -- a block RAM with one write port and two registered read ports.
//
// Every buffer of the decoder is one of these: the two spectra, the IMDCT
// work area, the overlap tails, the output samples. A read of the address
// being written in the same cycle returns the old word; no unit of the
// decoder reads a word in the cycle it writes it.

module aac_ram #(
    parameter WIDTH = 35,
    parameter DEPTH = 1024,
    parameter AW    = 10
) (
    input  wire             clk,
    input  wire             we,
    input  wire [AW-1:0]    waddr,
    input  wire [WIDTH-1:0] wdata,
    input  wire [AW-1:0]    raddr_a,
    output reg  [WIDTH-1:0] rdata_a,
    input  wire [AW-1:0]    raddr_b,
    output reg  [WIDTH-1:0] rdata_b
);
    reg [WIDTH-1:0] mem [0:DEPTH-1];
    always @(posedge clk) begin
        if (we)
            mem[waddr] <= wdata;
        rdata_a <= mem[raddr_a];
        rdata_b <= mem[raddr_b];
    end
endmodule
