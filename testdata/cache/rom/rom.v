// A ROM that $readmemh loads, for the discovered-input tests.
module rom (
    input  wire       clk,
    input  wire [2:0] addr,
    output reg  [7:0] data
);
    reg [7:0] mem [0:7];
    initial $readmemh("prog.hex", mem);
    always @(posedge clk) data <= mem[addr];
endmodule
