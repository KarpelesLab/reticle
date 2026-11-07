-- `=` against a constant holding an unknown, held to what Vivado 2026.1's
-- xsim printed for this file (unknown_compare_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;

entity tb is end entity;

architecture sim of tb is
  signal x : std_logic := 'X';
  signal z : std_logic := 'Z';
  signal g : std_logic_vector(3 downto 0) := "----";
  signal v : std_logic_vector(3 downto 0) := "10XZ";
begin
  process
  begin
    if ('1' ?= 'X') = 'X' then report "?= gives X"; end if;
    if x = 'X' then report "x = 'X'"; end if;
    if z = 'Z' and z /= 'X' then report "z = 'Z'"; end if;
    if g = "----" then report "g = ----"; end if;
    if v = "10XZ" then report "v = 10XZ"; end if;
    if v /= x"F" then report "v /= F"; end if;
    wait;
  end process;
end architecture;
