library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable c : integer := 0;
  begin
    outer : for a in 0 to 2 loop
      for b in 0 to 2 loop
        next outer when b > a;
        c := c + 1;
      end loop;
    end loop;
    if c = 6 then report "next outer ok"; else report "next outer bad"; end if;
    wait;
  end process;
end architecture;
