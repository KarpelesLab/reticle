// Each bit of a bus joined to an `inout` port of its own buffer, as a
// mapped netlist connects a bidirectional bus: `.IO(bus[k])`. The
// simulator once carried the buffer's drive out to the bus and nothing
// from the bus back in, so every buffer read `z` while the testbench drove
// the bus. Then both sides release, which a join made of two plain copies
// would fail by holding the last value.
module iobuf (output wire O, inout wire IO, input wire I, input wire T);
  assign IO = T ? 1'bz : I;
  assign O = IO;
endmodule
module inout_part_tb;
  reg [1:0] drive = 2'b10;
  reg en = 1, t = 1;
  wire [1:0] bus;
  wire [1:0] o;
  assign bus = en ? drive : 2'bzz;
  iobuf b0 (.O(o[0]), .IO(bus[0]), .I(1'b1), .T(t));
  iobuf b1 (.O(o[1]), .IO(bus[1]), .I(1'b0), .T(t));
  integer errors = 0;
  initial begin
    #1;
    if (o !== 2'b10) begin errors = errors + 1; $display("FAIL: driven from outside, read %b", o); end
    en = 0; t = 0; #1;
    if (bus !== 2'b01 || o !== 2'b01) begin errors = errors + 1; $display("FAIL: buffers drive, bus %b read %b", bus, o); end
    t = 1; #1;
    if (bus !== 2'bzz || o !== 2'bzz) begin errors = errors + 1; $display("FAIL: released, bus %b read %b", bus, o); end
    en = 1; drive = 2'b10; t = 0; #1;
    if (bus !== 2'bxx || o !== 2'bxx) begin errors = errors + 1; $display("FAIL: contention, bus %b read %b", bus, o); end
    if (errors == 0) $display("PASS: a bus joined to inout ports bit by bit resolves both ways");
  end
endmodule
