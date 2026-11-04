// A register file and an ALU in one stage: the critical path of a small CPU.
module regfile_alu (input wire clk, input wire si, output reg so);
  reg [20:0] in;
  reg [31:0] r [0:31];
  reg [31:0] y;
  wire [31:0] a = r[in[4:0]];
  wire [31:0] b = r[in[9:5]];
  always @* case (in[17:15])
    3'd0: y = a + b;
    3'd1: y = a - b;
    3'd2: y = a & b;
    3'd3: y = a | b;
    3'd4: y = a ^ b;
    3'd5: y = a << b[4:0];
    3'd6: y = a >> b[4:0];
    3'd7: y = {31'd0, $signed(a) < $signed(b)};
  endcase
  always @(posedge clk) begin
    in <= {in[19:0], si};
    if (in[20]) r[in[14:10]] <= y;
    so <= ^y;
  end
endmodule
