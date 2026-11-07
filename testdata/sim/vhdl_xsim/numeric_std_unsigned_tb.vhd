-- `ieee.numeric_std_unsigned`, held to what Vivado 2026.1's xsim printed
-- for this file (numeric_std_unsigned_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std_unsigned.all;

entity tb is end entity;

architecture sim of tb is
  signal s : std_logic_vector(7 downto 0) := x"30";
begin
  process
    variable a : std_logic_vector(7 downto 0) := x"F0";
    variable b : std_logic_vector(7 downto 0) := x"0F";
    variable c : std_logic_vector(15 downto 0);
    variable n : natural;
  begin
    report "a+b=" & to_hstring(a + b) & " a-b=" & to_hstring(a - b) & " a+1=" & to_hstring(a + 1);
    c := a * b;
    report "a*b=" & to_hstring(c) & " a/b=" & to_hstring(a / b) & " a rem 7=" & to_hstring(a rem 7);
    if a > b and b < 100 and a = 240 and s + 1 = x"31" then report "comparisons"; end if;
    if b = x"000F" then report "different widths compare as numbers"; end if;
    n := to_integer(a);
    report "to_integer=" & integer'image(n) & " to_slv=" & to_hstring(to_slv(200, 8))
      & " to_std_logic_vector=" & to_string(to_std_logic_vector(5, 4));
    report "shift " & to_hstring(shift_left(b, 4)) & " " & to_hstring(shift_right(a, 2))
      & " rotate " & to_hstring(rotate_left(a, 2)) & " resize " & to_hstring(resize(b, 12));
    report "min " & to_hstring(minimum(a, b)) & " max " & to_hstring(maximum(a, b));
    wait;
  end process;
end architecture;
