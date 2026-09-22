-- Static folding of `ieee.numeric_std`. Every constant below is computed
-- by the analyser through the native builtins, so the expected types file
-- is also a table of hand-checked answers.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package numeric_std_static is

  constant u200 : unsigned(7 downto 0) := to_unsigned(200, 8);
  constant u100 : unsigned(7 downto 0) := to_unsigned(100, 8);
  constant sm7  : signed(7 downto 0)   := to_signed(-7, 8);
  constant sp3  : signed(7 downto 0)   := to_signed(3, 8);

  -- Unsigned arithmetic, wrapping at 2**8.
  constant u_add   : unsigned(7 downto 0) := u200 + u100;
  constant u_sub   : unsigned(7 downto 0) := u100 - u200;
  constant u_div   : unsigned(7 downto 0) := u200 / u100;
  constant u_rem   : unsigned(7 downto 0) := u200 rem u100;
  constant u_incr  : unsigned(7 downto 0) := u200 + 55;
  constant u_mul   : unsigned(15 downto 0) := u200 * u100;
  constant u_wide  : unsigned(15 downto 0) := resize(u200, 16);
  constant u_narrow : unsigned(3 downto 0) := resize(u200, 4);

  -- Signed arithmetic. `rem` takes the sign of the dividend and `mod`
  -- the sign of the divisor, so the two differ here.
  constant s_add   : signed(7 downto 0) := sm7 + sp3;
  constant s_sub   : signed(7 downto 0) := sm7 - sp3;
  constant s_mul   : signed(15 downto 0) := sm7 * sp3;
  constant s_div   : signed(7 downto 0) := sm7 / sp3;
  constant s_rem   : signed(7 downto 0) := sm7 rem sp3;
  constant s_mod   : signed(7 downto 0) := sm7 mod sp3;
  constant s_neg   : signed(7 downto 0) := -sm7;
  constant s_abs   : signed(7 downto 0) := abs(sm7);
  constant s_wide  : signed(15 downto 0) := resize(sm7, 16);
  -- Narrowing a signed keeps the sign bit: "11111001" becomes "1001".
  constant s_narrow : signed(3 downto 0) := resize(sm7, 4);

  -- Comparison, including mixed widths and an integer operand.
  constant lt_u : boolean := u100 < u200;
  constant gt_s : boolean := sp3 > sm7;
  constant eq_i : boolean := u200 = 200;
  constant ne_i : boolean := sm7 /= 0;
  constant mix  : boolean := unsigned'("0001") < u200;

  -- The VHDL-2008 matching operators answer with a logic value.
  constant m_eq : std_ulogic := u200 ?= u200;
  constant m_ne : std_ulogic := u200 ?/= u100;
  constant m_lt : std_ulogic := u100 ?< u200;

  -- Shifts and rotates.
  constant sl : unsigned(7 downto 0) := shift_left(u100, 1);
  constant sr : unsigned(7 downto 0) := shift_right(u200, 4);
  constant sa : signed(7 downto 0)   := shift_right(sm7, 1);
  constant rl : unsigned(7 downto 0) := rotate_left(u200, 4);
  constant rr : unsigned(7 downto 0) := rotate_right(u200, 4);
  constant op_srl : signed(7 downto 0) := sm7 srl 1;

  -- Conversions back to integers.
  constant back_u : integer := to_integer(u200);
  constant back_s : integer := to_integer(sm7);

  -- Extrema, search and matching.
  constant mx : unsigned(7 downto 0) := maximum(u100, u200);
  constant mn : signed(7 downto 0)   := minimum(sm7, sp3);
  constant fl : integer := find_leftmost(u200, '1');
  constant fr : integer := find_rightmost(u200, '1');
  constant sx : boolean := std_match(u200, unsigned'("11--1000"));
  constant nx : boolean := is_x(u200);

  -- Element-wise logic and the 2008 reductions.
  constant l_and : unsigned(7 downto 0) := u200 and u100;
  constant l_or  : unsigned(7 downto 0) := u200 or u100;
  constant l_not : unsigned(7 downto 0) := not u200;
  constant r_or  : std_ulogic := or u200;
  constant r_and : std_ulogic := and u200;
  constant r_xor : std_ulogic := xor u200;

  -- Rendering. The strings themselves are unconstrained, so the checks
  -- below are what pins the text down.
  constant txt : string := to_string(u200);
  constant hex : string := to_hstring(u200);
  constant oct : string := to_ostring(u200);
  constant txt_ok : boolean := txt = "11001000";
  constant hex_ok : boolean := hex = "C8";
  constant oct_ok : boolean := oct = "310";

end package numeric_std_static;
