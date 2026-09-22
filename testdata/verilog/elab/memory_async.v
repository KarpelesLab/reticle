// A register file with two asynchronous read ports.
module memory_async(
    input  wire        clk,
    input  wire        we,
    input  wire [3:0]  waddr,
    input  wire [15:0] wdata,
    input  wire [3:0]  raddr_a,
    input  wire [3:0]  raddr_b,
    output wire [15:0] rdata_a,
    output wire [15:0] rdata_b
);
    reg [15:0] regs [15:0];

    always @(posedge clk)
        if (we) regs[waddr] <= wdata;

    assign rdata_a = regs[raddr_a];
    assign rdata_b = regs[raddr_b];
endmodule
