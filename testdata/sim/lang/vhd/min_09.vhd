library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    type c_t is (r, g, b);
    variable c : c_t;
  begin
    c := maximum(r, b);
    if c = b then report "max enum ok"; else report "max enum bad"; end if;
    wait;
  end process;
end architecture;
