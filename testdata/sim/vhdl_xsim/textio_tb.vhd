-- `std.textio` writes to the standard output, held to what Vivado 2026.1's
-- xsim printed for this file (textio_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.textio.all;

entity tb is end entity;

architecture sim of tb is
begin
  process
    variable l : line;
    variable v : std_logic_vector(7 downto 0) := x"3C";
    variable n : integer := 42;
    variable r : real := 3.25;
    variable t : time := 1500 ps;
  begin
    write(l, string'("hello from textio"));
    writeline(output, l);
    write(l, string'("n="));
    write(l, n);
    write(l, string'(" v="));
    write(l, v);
    write(l, string'(" h="));
    hwrite(l, v);
    write(l, string'(" o="));
    owrite(l, v);
    writeline(output, l);
    write(l, true);
    write(l, ' ');
    write(l, character'('Z'));
    write(l, ' ');
    write(l, bit_vector'("1011"));
    write(l, ' ');
    write(l, bit'('1'));
    write(l, ' ');
    write(l, std_logic'('X'));
    writeline(output, l);
    write(l, n, right, 6);
    write(l, string'("|"));
    write(l, n, left, 6);
    write(l, string'("|"));
    write(l, string'("ab"), right, 4);
    writeline(output, l);
    write(l, r);
    write(l, string'(" "));
    write(l, r, right, 0, 2);
    write(l, string'(" "));
    write(l, t);
    write(l, string'(" "));
    write(l, t, right, 0, ps);
    writeline(output, l);
    swrite(l, "swrite works");
    writeline(output, l);
    writeline(output, l);
    wait for 7 ns;
    write(l, string'("at "));
    write(l, now);
    writeline(output, l);
    wait;
  end process;
end architecture;
