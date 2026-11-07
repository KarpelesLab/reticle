library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal hi, lo : std_logic_vector(3 downto 0) := "1010";
begin
  process
    variable x : std_logic_vector(7 downto 0);
  begin
    x := (hi, lo);
    if x = x"AA" then report "vec agg ok"; else report "vec agg bad"; end if;
    wait;
  end process;
end architecture;
