/* VHDL-2008 delimited comments. */
entity /* inline */ d is
  /*
   * Multi-line, with Unicode: 日本語 🙂 and a -- inside,
   * and "quotes" and a nested-looking /* opener.
   */
  port (a : in bit /* comment between tokens */; b : out bit);
end;
x <= a /**/ and b; /* right after a token */
y <= a/*tight*/or b;
