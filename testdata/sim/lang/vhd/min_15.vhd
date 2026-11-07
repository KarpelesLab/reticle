library ieee;
use ieee.std_logic_1164.all;
use ieee.math_real.all;
entity tb is end entity;
architecture sim of tb is
  constant F : real := 100.0e6;
begin
  process
  begin
    wait for 1.0e9 / F * 1 ns;
    report "waited";
    wait;
  end process;
end architecture;
