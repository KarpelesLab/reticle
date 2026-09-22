-- `ieee.numeric_bit` is `numeric_std` over `bit`, and shares its native
-- core; only the element encoding differs, which these answers confirm.
library ieee;
use ieee.numeric_bit.all;

package numeric_bit_static is

  constant u200 : unsigned(7 downto 0) := to_unsigned(200, 8);
  constant u100 : unsigned(7 downto 0) := to_unsigned(100, 8);
  constant sm7  : signed(7 downto 0)   := to_signed(-7, 8);
  constant sp3  : signed(7 downto 0)   := to_signed(3, 8);

  constant u_add : unsigned(7 downto 0) := u200 + u100;
  constant u_mul : unsigned(15 downto 0) := u200 * u100;
  constant s_div : signed(7 downto 0) := sm7 / sp3;
  constant s_rem : signed(7 downto 0) := sm7 rem sp3;
  constant s_mod : signed(7 downto 0) := sm7 mod sp3;
  constant s_abs : signed(7 downto 0) := abs(sm7);
  constant narrow : signed(3 downto 0) := resize(sm7, 4);
  constant shifted : unsigned(7 downto 0) := shift_right(u200, 4);
  constant rotated : unsigned(7 downto 0) := rotate_left(u200, 4);
  constant back : integer := to_integer(sm7);
  constant less : boolean := sm7 < sp3;
  constant found : integer := find_leftmost(u200, '1');
  constant folded : bit := xor u200;
  constant text : string := to_string(u200);
  constant text_ok : boolean := text = "11001000";

end package numeric_bit_static;
