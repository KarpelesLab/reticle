-- The IEEE.STD_LOGIC_TEXTIO package.
--
-- Clean-room source for Reticle. This file is an original implementation
-- of the package interface defined by the relevant IEEE standard, written
-- for Reticle; it is not derived from the IEEE source distribution.
--
-- The package adds the `std.textio` read and write overloads for the
-- nine-state types. VHDL-2008 folded them into `ieee.std_logic_1164`
-- itself, but a great deal of code still names this package, so it is
-- bundled and declares the same profiles. Every subprogram touches the
-- host's streams, so each is `attribute foreign` and implemented by the
-- simulator; the package has no body.
--
-- Since the 2008 revision, `std_logic_vector` is a subtype of
-- `std_ulogic_vector` rather than a distinct type, so an overload named
-- for it would be a homograph of the `std_ulogic_vector` one. The
-- declarations below therefore cover both spellings.

library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;

package std_logic_textio is

  procedure read (l : inout line; value : out std_ulogic; good : out boolean);
  procedure read (l : inout line; value : out std_ulogic);
  procedure read (l : inout line; value : out std_ulogic_vector; good : out boolean);
  procedure read (l : inout line; value : out std_ulogic_vector);

  procedure write (l         : inout line;
                   value     : in std_ulogic;
                   justified : in side  := right;
                   field     : in width := 0);
  procedure write (l         : inout line;
                   value     : in std_ulogic_vector;
                   justified : in side  := right;
                   field     : in width := 0);

  procedure hread (l : inout line; value : out std_ulogic_vector; good : out boolean);
  procedure hread (l : inout line; value : out std_ulogic_vector);
  procedure oread (l : inout line; value : out std_ulogic_vector; good : out boolean);
  procedure oread (l : inout line; value : out std_ulogic_vector);
  procedure bread (l : inout line; value : out std_ulogic_vector; good : out boolean);
  procedure bread (l : inout line; value : out std_ulogic_vector);

  procedure hwrite (l         : inout line;
                    value     : in std_ulogic_vector;
                    justified : in side  := right;
                    field     : in width := 0);
  procedure owrite (l         : inout line;
                    value     : in std_ulogic_vector;
                    justified : in side  := right;
                    field     : in width := 0);
  procedure bwrite (l         : inout line;
                    value     : in std_ulogic_vector;
                    justified : in side  := right;
                    field     : in width := 0);

  attribute foreign of read   : procedure is "reticle: builtin";
  attribute foreign of write  : procedure is "reticle: builtin";
  attribute foreign of hread  : procedure is "reticle: builtin";
  attribute foreign of oread  : procedure is "reticle: builtin";
  attribute foreign of bread  : procedure is "reticle: builtin";
  attribute foreign of hwrite : procedure is "reticle: builtin";
  attribute foreign of owrite : procedure is "reticle: builtin";
  attribute foreign of bwrite : procedure is "reticle: builtin";

end package std_logic_textio;
