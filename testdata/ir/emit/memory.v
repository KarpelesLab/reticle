module ram (
  input wire clk,
  input wire we,
  input wire [3:0] addr,
  input wire [7:0] wdata,
  output reg [7:0] rdata,
  output wire [7:0] rom_out
);
  (* ram_style = "block" *)
  reg [7:0] mem [0:15];
  reg [7:0] rom [0:3];
  initial begin
    rom[0] = 8'hde;
    rom[1] = 8'had;
    rom[2] = 8'hbe;
    rom[3] = 8'hef;
  end
  assign rom_out = rom[addr[1:0]];
  always @(posedge clk) begin
    if (we) mem[addr] <= wdata;
    rdata <= mem[addr];
  end
endmodule
