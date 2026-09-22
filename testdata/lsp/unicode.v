// This file holds a two-byte character (é), a three-byte one (中) and a
// four-byte one (😀, two UTF-16 code units), so the columns in
// unicode.session are not the byte offsets of the same tokens.
module unicode (
  input  wire [7:0] data,
  output wire [7:0] echo
);
  assign echo = data;

  initial $display("é 中 😀 = %d", data);
endmodule
