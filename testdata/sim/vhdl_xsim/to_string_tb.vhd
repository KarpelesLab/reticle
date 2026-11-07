-- The two-argument `to_string` overloads, `minimum`/`maximum` of a vector,
-- a unit name as a value, and a loop over an enumeration range written
-- with attributes, held to what Vivado 2026.1's xsim printed for this file
-- (to_string_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;

entity tb is end entity;

architecture sim of tb is
  type color_t is (red, green, blue, yellow);
begin
  process
    variable t : time := 1500 ps;
    variable r : real := 3.14159;
    variable a : std_logic_vector(3 downto 0) := "0110";
    variable b : std_logic_vector(3 downto 0) := "0011";
  begin
    report to_string(t, ps) & " " & to_string(t, ns) & " " & to_string(2 ns, ps)
      & " " & to_string(now, fs);
    report to_string(r, 2) & " " & to_string(r, "%.3f") & " " & to_string(r, "%e")
      & " " & to_string(r, 0);
    report "min " & to_string(minimum(a, b)) & " max " & to_string(maximum(a, b));
    report "unit " & time'image(ns) & " " & integer'image(t / ps);
    for k in color_t'high downto color_t'succ(color_t'low) loop
      report "colour " & color_t'image(k);
    end loop;
    wait;
  end process;
end architecture;
