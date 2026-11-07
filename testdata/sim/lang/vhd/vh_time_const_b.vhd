library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  type distance is range 0 to 100000000
    units
      mm;
      cm = 10 mm;
      m = 100 cm;
      km = 1000 m;
    end units;
  constant period : time := 10 ns;
  signal t_sig : time := 0 ns;
  signal clk_cnt : integer := 0;
begin
  process
    variable t : time;
    variable d : distance;
  begin
    wait for 5 ns;
    t := period * 3;
    if t = 30 ns then report "mul ok"; else report "mul bad"; end if;
    t := period / 4;
    if t = 2500 ps then report "div ok"; else report "div bad"; end if;
    if period / 1 ns = 10 then report "ratio ok"; else report "ratio bad"; end if;
    if 1.5 ns = 1500 ps then report "real lit ok"; else report "real lit bad"; end if;
    if 1 us > 999 ns then report "cmp ok"; else report "cmp bad"; end if;
    if 1 ms + 1 us = 1001 us then report "add ok"; else report "add bad"; end if;
    d := 2 km + 5 cm;
    if d / 1 mm = 2000050 then report "phys ok"; else report "phys bad"; end if;
    if d / 1 m = 2000 then report "phys m ok"; else report "phys m bad"; end if;
    wait for period;
    wait for 2 ns;
    if t_sig = 0 ns then report "not yet ok"; else report "not yet bad"; end if;
    wait for 2 ns;
    if t_sig = 15 ns then report "t_sig ok"; else report "t_sig bad"; end if;
    wait for 1.5 ns;
    wait;
  end process;
end architecture;
