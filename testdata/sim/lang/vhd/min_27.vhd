library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  type down_arr is array (7 downto 4) of integer;
begin
  process
    variable d : down_arr := (1, 2, 3, 4);
  begin
    if d(7) = 1 then report "d(7)=1 ok"; elsif d(7) = 4 then report "d(7)=4 reversed"; else report "d(7) other"; end if;
    if d(4) = 4 then report "d(4)=4 ok"; elsif d(4) = 1 then report "d(4)=1 reversed"; else report "d(4) other"; end if;
    wait;
  end process;
end architecture;
