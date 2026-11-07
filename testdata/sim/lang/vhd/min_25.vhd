library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  type int_arr is array (0 to 3) of integer;
begin
  process
    variable ia : int_arr;
  begin
    ia := (others => 7);
    if ia(2) = 7 then report "arr agg assign ok"; else report "arr agg assign bad"; end if;
    wait;
  end process;
end architecture;
