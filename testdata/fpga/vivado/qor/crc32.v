// A 32-bit CRC taken over 32 bits at a time: deep XOR logic.
module crc32 (input wire clk, input wire si, output reg so);
  reg [31:0] in, crc, c;
  integer i;
  always @* begin
    c = crc ^ in;
    for (i = 0; i < 32; i = i + 1)
      c = c[0] ? (c >> 1) ^ 32'hEDB88320 : c >> 1;
  end
  always @(posedge clk) begin
    in <= {in[30:0], si};
    crc <= c;
    so <= ^crc;
  end
endmodule
