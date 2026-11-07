-- `real` variables and signals, `ieee.math_real` at run time, and the
-- conversions, held to what Vivado 2026.1's xsim printed for this file
-- (reals_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;
use ieee.math_real.all;

entity tb is end entity;

architecture sim of tb is
  constant F : real := 125.0e6;
  signal rs : real := 0.5;
begin
  rs <= 1.75 after 2 ns;

  process
    variable x : real := 2.5;
    variable y : real := -1.25;
    variable i : integer := 7;
    variable t : time := 10 ns;
  begin
    report "x=" & real'image(x) & " y=" & real'image(y) & " rs=" & real'image(rs);
    report "sum " & real'image(x + y) & " prod " & real'image(x * y)
      & " quot " & real'image(x / y) & " pow " & real'image(x ** 2);
    report "neg " & real'image(-x) & " abs " & real'image(abs y)
      & " mod " & real'image(x mod 1.0);
    if x > y and not (x = y) and x * 2.0 = 5.0 then report "comparisons"; end if;
    report "sqrt " & real'image(sqrt(x)) & " floor " & real'image(floor(y))
      & " ceil " & real'image(ceil(y)) & " round " & real'image(round(x));
    report "log2 " & real'image(log2(real(1024))) & " exp " & real'image(exp(1.0))
      & " sin " & real'image(sin(MATH_PI / 2.0)) & " max " & real'image(realmax(x, y));
    report "integer(x) " & integer'image(integer(x)) & " integer(y) " & integer'image(integer(y))
      & " real(i) " & real'image(real(i)) & " to_string " & to_string(x);
    report "time*real " & time'image(t * 1.5) & " real*time " & time'image(0.25 * t);
    wait for 1.0e9 / F * 1 ns;
    report "waited until " & time'image(now);
    wait for 2 ns;
    report "rs now " & real'image(rs);
    wait;
  end process;
end architecture;
