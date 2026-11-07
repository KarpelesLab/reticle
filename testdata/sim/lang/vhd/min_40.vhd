library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal s : integer := 0;
begin
  process
  begin
    s <= 5;
    wait for 0 ns;
    if s = 5 then report "wait 0 ns updates ok"; elsif s = 0 then report "wait 0 ns: still 0"; else report "other"; end if;
    wait for 1 ns;
    if s = 5 then report "after 1 ns 5"; else report "after 1 ns not 5"; end if;
    s <= 1;
    s <= 2;
    s <= 3;
    wait for 1 ns;
    if s = 3 then report "last wins ok"; elsif s = 1 then report "first wins"; else report "last wins other"; end if;
    wait;
  end process;
end architecture;
