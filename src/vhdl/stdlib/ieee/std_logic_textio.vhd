-- The IEEE.STD_LOGIC_TEXTIO package.
--
-- Clean-room source for Reticle. This file is an original implementation
-- of the package interface defined by the relevant IEEE standard, written
-- for Reticle; it is not derived from the IEEE source distribution.
--
-- The package held the `std.textio` read and write overloads for the
-- nine-state types. VHDL-2008 folded them into `ieee.std_logic_1164`
-- itself and left this package empty, and so does Reticle: a design that
-- names it finds the overloads in `ieee.std_logic_1164`, which it uses for
-- the types anyway. (It declared the same profiles here, which made a
-- design using both packages see two homographs of every `write`.)

library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;

package std_logic_textio is
end package std_logic_textio;
