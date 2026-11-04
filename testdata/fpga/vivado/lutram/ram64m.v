// A RAM64M by name, reading ports abcd.
module ram64m (
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
  RAM64M #(.INIT_A(64'h29c5547d8706823d), .INIT_B(64'hf0557fdba87619e5), .INIT_C(64'heb0a1e1cdbf50468), .INIT_D(64'h9713a7e6c6060caa)) dut (.ADDRA(sr[5:0]), .ADDRB(sr[11:6]), .ADDRC(sr[17:12]), .ADDRD(sr[23:18]), .DIA(sr[24]), .DIB(sr[25]), .DIC(sr[26]), .DID(sr[27]), .DOA(doa), .DOB(dob), .DOC(doc), .DOD(dod), .WE(sr[28]), .WCLK(clk));
  always @(posedge clk) so <= ^doa ^ ^dob ^ ^doc ^ ^dod;
endmodule
