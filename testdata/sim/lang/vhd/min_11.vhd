library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal a : std_logic := '0';
begin
  process
  begin
    if rising_edge(a) then report "edge"; else report "no edge"; end if;
    a <= '1';
    wait on a;
    if rising_edge(a) then report "edge"; else report "no edge"; end if;
    wait;
  end process;
end architecture;
