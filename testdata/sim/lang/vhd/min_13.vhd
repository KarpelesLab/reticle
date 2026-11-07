library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable t : time;
  begin
    wait for 5 ns;
    t := now;
    if t = 5 ns then report "now ok"; else report "now bad"; end if;
    wait;
  end process;
end architecture;
