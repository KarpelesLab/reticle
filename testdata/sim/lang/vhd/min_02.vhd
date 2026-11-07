library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable i : integer := 5;
  begin
    report integer'image(i);
    report "i=" & integer'image(i);
    wait;
  end process;
end architecture;
