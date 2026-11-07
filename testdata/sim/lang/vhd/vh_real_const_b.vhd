library ieee;
use ieee.std_logic_1164.all;
use ieee.math_real.all;

entity tb is
end entity;

architecture sim of tb is
  -- real used only in constants and generics-style elaboration expressions
  constant X : real := 2.5;
  constant W : integer := integer(ceil(log2(real(200))));
  constant R1 : integer := integer(3.5);
  constant R2 : integer := integer(-2.5);
  constant R3 : integer := integer(round(2.5));
  constant SQ : integer := integer(sqrt(16.0));
  constant FL : integer := integer(floor(-1.25));
  constant CL : integer := integer(ceil(-1.25));
  constant TR : integer := integer(trunc(-1.75));
  constant FREQ : real := 100.0e6;
  constant PER_NS : integer := integer(1.0e9 / FREQ);
  constant DIVR : integer := integer(50.0e6 / 9600.0);
begin
  process
  begin
    if W = 8 then report "clog2 ok"; else report "clog2 bad"; end if;
    if R1 = 4 then report "round half up ok"; else report "round half up bad"; end if;
    if R2 = -3 then report "round half neg ok"; else report "round half neg bad"; end if;
    if R3 = 3 then report "round ok"; else report "round bad"; end if;
    if SQ = 4 then report "sqrt ok"; else report "sqrt bad"; end if;
    if FL = -2 then report "floor ok"; else report "floor bad"; end if;
    if CL = -1 then report "ceil ok"; else report "ceil bad"; end if;
    if TR = -1 then report "trunc ok"; else report "trunc bad"; end if;
    if PER_NS = 10 then report "period ok"; else report "period bad"; end if;
    if DIVR = 5208 then report "baud ok"; else report "baud bad"; end if;
    if X * 2.0 = 5.0 then report "real cmp ok"; else report "real cmp bad"; end if;
    if X > 2.4 then report "real gt ok"; else report "real gt bad"; end if;
    if integer(MATH_PI * 1000.0) = 3142 then report "pi ok"; else report "pi bad"; end if;
    if integer(exp(1.0) * 1000.0) = 2718 then report "exp ok"; else report "exp bad"; end if;
    if integer(sin(MATH_PI / 6.0) * 100.0) = 50 then report "sin ok"; else report "sin bad"; end if;
    wait;
  end process;
end architecture;
