library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal s : std_logic := '0';
begin
  process
  begin
    s <= '1';
    wait for 0 ns;
    if s'event then report "event ok"; else report "event bad"; end if;
    if s'last_value = '0' then report "last_value ok"; else report "last_value bad"; end if;
    wait;
  end process;
end architecture;
