// A RAM32M by name, reading ports abcd.
module ram32m (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [28:0] sr;
  always @(posedge clk) sr <= {sr[27:0], si};
  wire [1:0] doa;
  wire [1:0] dob;
  wire [1:0] doc;
  wire [1:0] dod;
  RAM32M #(.INIT_A(64'hb69465836a5113b2), .INIT_B(64'h16429eededd73c45), .INIT_C(64'h0e96075cd64778b2), .INIT_D(64'h28aa431dbe025132)) dut (.ADDRA(sr[4:0]), .ADDRB(sr[9:5]), .ADDRC(sr[14:10]), .ADDRD(sr[19:15]), .DIA(sr[21:20]), .DIB(sr[23:22]), .DIC(sr[25:24]), .DID(sr[27:26]), .DOA(doa), .DOB(dob), .DOC(doc), .DOD(dod), .WE(sr[28]), .WCLK(clk));
  always @(posedge clk) so <= ^doa ^ ^dob ^ ^doc ^ ^dod;
endmodule
