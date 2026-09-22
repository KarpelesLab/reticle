// A simple synchronous RAM / ROM initialised from a hex file.
module rom #(
    parameter AW = 10,
    parameter DW = 32,
    parameter INIT_FILE = "boot.hex"
) (
    input  wire          clk,
    input  wire [AW-1:0] addr,
    output reg  [DW-1:0] data
);
    (* ram_style = "block" *)
    reg [DW-1:0] mem [0:(1<<AW)-1];

    initial begin
        if (INIT_FILE != "")
            $readmemh(INIT_FILE, mem);
        else
            $readmemb("zeros.bin", mem, 0, 15);
    end

    always @(posedge clk)
        data <= mem[addr];
endmodule

module ram_dp (
    input  wire        clk,
    input  wire        we,
    input  wire [7:0]  waddr, raddr,
    input  wire [15:0] wdata,
    output reg  [15:0] rdata
);
    reg [15:0] mem [255:0];
    integer i;

    initial
        for (i = 0; i < 256; i = i + 1)
            mem[i] = 16'h0000;

    always @(posedge clk) begin
        if (we) mem[waddr] <= wdata;
        rdata <= mem[raddr];
    end
endmodule
