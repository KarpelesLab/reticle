library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal c : integer := 0;
  procedure pulse(signal s : inout integer; n : in natural) is
  begin
    for i in 1 to n loop
      s <= s + 1;
      wait for 1 ns;
    end loop;
  end procedure;
begin
  process
  begin
    pulse(c, 3);
    if c = 3 then report "param loop bound ok"; else report "param loop bound bad"; end if;
    wait;
  end process;
end architecture;
