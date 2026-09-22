-- Subtypes with range, index and resolution constraints.
library ieee;
use ieee.std_logic_1164.all;

package sub_pkg is
  subtype byte_t   is natural range 0 to 255;
  subtype nibble_t is byte_t range 0 to 15;
  subtype word_t   is std_logic_vector(31 downto 0);
  subtype half_t   is word_t;
  subtype flag_t   is std_ulogic;
  subtype rflag_t  is resolved std_ulogic;

  constant MAXB : byte_t   := 255;
  constant MAXN : nibble_t := 15;
end package sub_pkg;

use work.sub_pkg.all;
library ieee;
use ieee.std_logic_1164.all;

entity sub_user is
  port (
    n : in  nibble_t;
    w : in  word_t;
    b : out byte_t;
    f : out rflag_t
  );
end entity;

architecture rtl of sub_user is
  signal acc : byte_t := 0;
begin
  acc <= n * 2;
  b   <= acc;
  f   <= w(0);
end architecture;
