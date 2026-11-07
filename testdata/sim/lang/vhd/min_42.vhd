library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal clk : std_logic := '1';
  signal other : integer := 0;
  signal flag : integer := 0;
begin
  process
  begin
    other <= 1 after 3 ns;
    clk <= '0' after 10 ns, '1' after 20 ns;
    wait until clk = '1';


    if other = 1 then report "resumed after 3 ns ok"; else report "resumed before any event"; end if;
    wait;
  end process;
end architecture;
