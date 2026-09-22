-- The Synopsys legacy packages, over the same native core. The two
-- `std_logic_vector` packages differ only in the signedness they impose,
-- which is why `conv_integer` of the same bits gives 200 in one and -56
-- in the other.
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_arith.all;

package arith_static is

  constant u200 : unsigned(7 downto 0) := conv_unsigned(200, 8);
  constant sm7  : signed(7 downto 0)   := conv_signed(-7, 8);

  constant u_add  : unsigned(7 downto 0) := u200 + 55;
  constant s_add  : signed(7 downto 0)   := sm7 + 3;
  constant s_abs  : signed(7 downto 0)   := abs(sm7);
  constant back_u : integer := conv_integer(u200);
  constant back_s : integer := conv_integer(sm7);
  constant as_slv : std_logic_vector(7 downto 0) := conv_std_logic_vector(sm7, 8);
  constant zeroed : std_logic_vector(11 downto 0) := ext(as_slv, 12);
  constant signd  : std_logic_vector(11 downto 0) := sxt(as_slv, 12);
  constant bigger : boolean := u200 > 100;

end package arith_static;

library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_unsigned.all;

package unsigned_static is
  constant bits : std_logic_vector(7 downto 0) := "11001000";
  constant as_int : integer := conv_integer(bits);
  constant plus : std_logic_vector(7 downto 0) := bits + 1;
  constant over : boolean := bits > "01111111";
end package unsigned_static;

library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_signed.all;

package signed_static is
  constant bits : std_logic_vector(7 downto 0) := "11001000";
  constant as_int : integer := conv_integer(bits);
  constant plus : std_logic_vector(7 downto 0) := bits + 1;
  constant under : boolean := bits < "01111111";
  constant magnitude : std_logic_vector(7 downto 0) := abs(bits);
end package signed_static;
