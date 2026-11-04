// The design every BITSTREAM.* option is written against: small, so that its
// frames can be committed as an answer, with some logic, a clock, two pins and
// a block RAM with something in it, so that both configuration buses carry data.
module options_probe (input wire clk, input wire si, output reg so);
    reg [7:0] count = 8'h5a;
    reg [7:0] rom [0:255];
    reg [7:0] dout;
    integer i;
    initial for (i = 0; i < 256; i = i + 1) rom[i] = (i * 37) ^ (i >> 3);
    always @(posedge clk) begin
        count <= si ? 8'h00 : count + 8'd1;
        dout <= rom[count];
        so <= ^dout;
    end
endmodule
