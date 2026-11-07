library ieee;
use ieee.std_logic_1164.all;
use ieee.math_real.all;

entity tb is
end entity;

architecture sim of tb is
begin
  process
    variable x : real := 2.5;
    variable y : real := -1.25;
    variable i : integer;
  begin
    report "x=" & real'image(x);
    report "y=" & real'image(y);
    report "x+y=" & real'image(x + y);
    report "x*y=" & real'image(x * y);
    report "x/y=" & real'image(x / y);
    report "x**2=" & real'image(x ** 2);
    report "sqrt(2)=" & real'image(sqrt(2.0));
    report "floor(-1.25)=" & real'image(floor(y));
    report "ceil(-1.25)=" & real'image(ceil(y));
    report "round(2.5)=" & real'image(round(x));
    report "round(-2.5)=" & real'image(round(-2.5));
    report "trunc(-1.75)=" & real'image(trunc(-1.75));
    report "realmax(1,2)=" & real'image(realmax(1.0, 2.0));
    report "log2(1024)=" & real'image(log2(1024.0));
    report "exp(1)=" & real'image(exp(1.0));
    report "ln(10)=" & real'image(log(10.0));
    report "sin(pi/2)=" & real'image(sin(MATH_PI / 2.0));
    report "cos(0)=" & real'image(cos(0.0));
    report "pi=" & real'image(MATH_PI);
    report "integer(2.5)=" & integer'image(integer(x));
    report "integer(3.5)=" & integer'image(integer(3.5));
    report "integer(-2.5)=" & integer'image(integer(-2.5));
    report "integer(2.4999)=" & integer'image(integer(2.4999));
    report "real(7)=" & real'image(real(7));
    report "ceil(log2(5))=" & integer'image(integer(ceil(log2(5.0))));
    report "sign(-3)=" & real'image(sign(-3.0));
    report "1.0e10=" & real'image(1.0e10);
    report "0.1=" & real'image(0.1);
    report "1.0/3.0=" & real'image(1.0 / 3.0);
    report "x>y " & boolean'image(x > y);
    report "abs y=" & real'image(abs y);
    report "x mod real=" & real'image(x mod 1.0);
    i := integer(floor(log2(real(1000))));
    report "floor log2 1000=" & integer'image(i);
    wait;
  end process;
end architecture;
