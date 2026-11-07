library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable l : line;
  begin
    write(l, string'("textio line"));
    writeline(output, l);
    wait;
  end process;
end architecture;
