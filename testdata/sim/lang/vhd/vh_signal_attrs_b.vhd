library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal clk : std_logic := '0';
  signal r1, r2, f1 : integer := 0;
begin
  -- 'event only as part of the clock condition of a process
  p1 : process (clk)
  begin
    if clk'event and clk = '1' then
      r1 <= r1 + 1;
    end if;
  end process;

  p2 : process (clk)
  begin
    if clk = '1' and clk'event then
      r2 <= r2 + 1;
    end if;
  end process;

  p3 : process (clk)
  begin
    if falling_edge(clk) then
      f1 <= f1 + 1;
    end if;
  end process;

  process
  begin
    for i in 1 to 3 loop
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
    end loop;
    wait for 1 ns;
    if r1 = 3 then report "event and level ok"; else report "event and level bad"; end if;
    if r2 = 3 then report "level and event ok"; else report "level and event bad"; end if;
    if f1 = 3 then report "falling_edge ok"; else report "falling_edge bad"; end if;
    wait;
  end process;
end architecture;
