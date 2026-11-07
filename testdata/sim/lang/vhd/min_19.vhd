library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  type mat_t is array (0 to 1, 0 to 3) of integer;
begin
  process
    variable m : mat_t := (others => (others => 0));
  begin
    m(1, 3) := 13;
    if m(1, 3) = 13 then report "2d ok"; else report "2d bad"; end if;
    wait;
  end process;
end architecture;
