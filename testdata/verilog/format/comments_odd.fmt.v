// A file whose comments sit in every awkward place a formatter has to
// cope with.

/* a block comment
   spanning several lines,
   kept exactly as written */
module odd (
  // after the open paren
  input  wire a, // after a port
  /* before b */
  input  wire b,
  output wire y
); // after the header
  // a leading comment, after a blank line
  wire t; // a trailing comment

  // three blank lines above collapse to one
  assign t = a | b;
  assign y = a & b; /* inside an expression */

  // dangling at the end of the module
endmodule // after endmodule

// the last word
