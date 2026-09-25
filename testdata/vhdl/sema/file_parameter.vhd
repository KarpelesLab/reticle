-- A file parameter has no mode of its own (clause 4.2.2.1), so passing one
-- on to another subprogram's file parameter is neither a read nor a write.
-- It was reported as an assignment to an `in` parameter, which is how every
-- file utility in Colibri's `binaryio` is written.
--
-- Found by the Colibri corpus; see docs/vhdl-corpus.md.
package io_pkg is

  type byte_file is file of character;

  procedure read_byte (file f : byte_file; c : out character);
  procedure read_two (file f : byte_file; a, b : out character);

end package io_pkg;

package body io_pkg is

  procedure read_byte (file f : byte_file; c : out character) is
  begin
    read(f, c);
  end procedure;

  procedure read_two (file f : byte_file; a, b : out character) is
  begin
    -- `f` is passed on, and `a` is passed to an `out` formal.
    read_byte(f, a);
    read_byte(f, b);
    -- VHDL-2008 allows an `out` parameter to be read as well
    -- (clause 4.2.2.3); before the first assignment its value is its
    -- subtype's default.
    if a = b then
      b := a;
    end if;
  end procedure;

end package body io_pkg;
