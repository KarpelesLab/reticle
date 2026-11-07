library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal a : std_logic := '0';
begin
  process
  begin
    report "before";
    wait until a = '1' for 7 ns;
    report "after timeout";
    wait;
  end process;
end architecture;
