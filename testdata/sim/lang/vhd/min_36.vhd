library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable s : integer := 0;
  begin
    for k in 1 to 3 loop
      for k2 in k to 3 loop
        s := s + 1;
      end loop;
    end loop;
    if s = 6 then report "dependent range ok"; else report "dependent range bad"; end if;
    wait;
  end process;
end architecture;
