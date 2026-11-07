library ieee;
use ieee.std_logic_1164.all;
use ieee.math_real.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable x : real := 2.5;
  begin
    if x * 2.0 = 5.0 then report "real ok"; else report "real bad"; end if;
    wait;
  end process;
end architecture;
