library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal src, inert : std_logic := '0';
  signal w : integer := 0;
begin
  inert <= src after 3 ns;
  process
  begin
    wait for 1 ns;
    src <= '1';
    wait for 2 ns;
    src <= '0';
    wait for 1 ns;
    if inert = '0' then report "inertial swallows 2ns pulse ok"; else report "inertial passed 2ns pulse"; end if;
    w <= 10 after 2 ns;
    wait for 1 ns;
    w <= 20 after 3 ns;
    wait for 1500 ps;
    if w = 0 then report "pending 10 cancelled ok"; elsif w = 10 then report "pending 10 not cancelled"; else report "w other"; end if;
    wait;
  end process;
end architecture;
