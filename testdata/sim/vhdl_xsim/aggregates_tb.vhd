-- Aggregates whose elements are vectors, `'value`, and a logical operator
-- of a scalar and a vector, held to what Vivado 2026.1's xsim printed for
-- this file (aggregates_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;

entity tb is end entity;

architecture sim of tb is
  signal hi : std_logic_vector(3 downto 0) := "1010";
  signal lo : std_logic_vector(3 downto 0) := "0101";
begin
  process
    variable x : std_logic_vector(7 downto 0);
    variable y : std_logic_vector(9 downto 0);
    variable b : std_logic_vector(7 downto 0) := x"5A";
  begin
    x := (hi, lo);
    report "x=" & to_hstring(x);
    y := ('1', hi, "0", lo);
    report "y=" & to_string(y);
    x := (lo(1 downto 0), hi(3 downto 2), "1111");
    report "x=" & to_string(x);
    report "value " & integer'image(integer'value("  -42")) & " " & integer'image(integer'value("17"));
    report "or " & to_string('0' or b) & " and " & to_string(b and '1') & " xor " & to_string('1' xor b);
    wait;
  end process;
end architecture;
