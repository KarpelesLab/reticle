-- top: tb
-- A testbench whose timing comes from `ieee.math_real`: the clock period
-- is computed from a frequency in hertz, and the table of phase steps
-- from a sine. Every call folds in the analyser, so the lowered design
-- holds the numbers rather than the arithmetic.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.math_real.all;

entity tb is
end entity;

architecture sim of tb is
  constant clock_hz : real := 50.0e6;
  constant period   : time := integer(round(1.0e9 / clock_hz)) * 1 ns;
  constant half     : time := period / 2;

  -- A quarter turn of a sine, scaled to a signed byte.
  constant q0 : integer := integer(round(127.0 * sin(0.0 * math_pi_over_4)));
  constant q1 : integer := integer(round(127.0 * sin(1.0 * math_pi_over_4)));
  constant q2 : integer := integer(round(127.0 * sin(2.0 * math_pi_over_4)));

  signal clk   : std_logic := '0';
  signal phase : signed(7 downto 0) := (others => '0');
begin
  clocking : process
  begin
    for i in 0 to 3 loop
      wait for half;
      clk <= '1';
      wait for half;
      clk <= '0';
    end loop;
    wait;
  end process;

  stimulus : process
  begin
    report "start";
    assert period = 20 ns report "the period is wrong" severity error;
    assert q0 = 0 report "sin(0) is wrong" severity error;
    assert q1 = 90 report "sin(pi/4) is wrong" severity error;
    assert q2 = 127 report "sin(pi/2) is wrong" severity error;
    assert sqrt(2.0) > 1.414 and sqrt(2.0) < 1.415
      report "sqrt(2) is wrong" severity error;
    assert log2(1024.0) = 10.0 report "log2 is wrong" severity error;
    phase <= to_signed(q1, 8);
    wait for period;
    assert to_integer(phase) = 90 report "the phase step is wrong" severity error;
    report "done";
    wait;
  end process;
end architecture;
