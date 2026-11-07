library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;

entity tb is
end entity;

architecture sim of tb is
begin
  process
    variable l : line;
    variable v : std_logic_vector(7 downto 0) := x"3C";
    variable n : integer := 42;
  begin
    write(l, string'("hello from textio"));
    writeline(output, l);
    write(l, string'("n="));
    write(l, n);
    writeline(output, l);
    writeline(output, l);
    writeline(output, l);
    writeline(output, l);
    write(l, string'("bool="));
    write(l, true);
    writeline(output, l);
    write(l, string'("char="));
    write(l, character'('Z'));
    writeline(output, l);
    write(l, string'("time="));
    write(l, 25 ns);
    writeline(output, l);
    write(l, n, right, 6);
    write(l, string'("|"));
    write(l, n, left, 6);
    write(l, string'("|"));
    writeline(output, l);
    write(l, string'("bit_vector="));
    write(l, bit_vector'("1011"));
    writeline(output, l);
    writeline(output, l);
    write(l, string'("real="));
    write(l, 3.25);
    writeline(output, l);
    writeline(output, l);
    write(l, string'(""));
    writeline(output, l);
    wait for 7 ns;
    write(l, string'("at "));
    write(l, now);
    writeline(output, l);
    wait;
  end process;
end architecture;
