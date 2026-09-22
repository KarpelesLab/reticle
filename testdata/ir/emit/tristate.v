module io_pad (
  input wire oe,
  input wire [3:0] d,
  inout wire [3:0] pad,
  output wire [3:0] din,
  input wire \bit ,
  inout wire bit_pad
);
  assign pad = oe ? d : {4{1'bz}};
  assign din = pad;
  assign bit_pad = oe ? \bit  : 1'bz;
endmodule
