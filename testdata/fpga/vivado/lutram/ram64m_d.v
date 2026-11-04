// A RAM64M d by name, reading ports d.
module ram64m_d (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [28:0] sr;
  always @(posedge clk) sr <= {sr[27:0], si};
  wire [0:0] doa;
  wire [0:0] dob;
  wire [0:0] doc;
  wire [0:0] dod;
  RAM64M #(.INIT_A(64'h15add806414aef36), .INIT_B(64'h535dc408d2267cf7), .INIT_C(64'h6ab210718b6a1ec2), .INIT_D(64'hf079ceef0abe3659)) dut (.ADDRA(sr[5:0]), .ADDRB(sr[11:6]), .ADDRC(sr[17:12]), .ADDRD(sr[23:18]), .DIA(sr[24]), .DIB(sr[25]), .DIC(sr[26]), .DID(sr[27]), .DOA(doa), .DOB(dob), .DOC(doc), .DOD(dod), .WE(sr[28]), .WCLK(clk));
  always @(posedge clk) so <= ^dod;
endmodule
