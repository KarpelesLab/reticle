library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal s : std_logic := '0';
  signal d : std_logic;
begin
  d <= s'delayed(2 ns);
  process
  begin
    s <= '1';
    wait for 1 ns;
    if d = '0' then report "delayed before ok"; else report "delayed before bad"; end if;
    wait for 2 ns;
    if d = '1' then report "delayed after ok"; else report "delayed after bad"; end if;
    wait;
  end process;
end architecture;
