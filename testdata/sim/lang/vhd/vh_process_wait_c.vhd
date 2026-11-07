library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal a, b : std_logic := '0';
  signal y_list, y_all : std_logic;
  signal y_incomplete : std_logic := '0';
  signal clk : std_logic := '0';
  signal done : boolean := false;
  signal cnt : integer := 0;
  signal ev : integer := 0;
begin
  p_list : process (a, b)
  begin
    y_list <= a xor b;
  end process;

  p_all : process (all)
  begin
    y_all <= a and b;
  end process;

  -- deliberately incomplete sensitivity list: only a
  p_inc : process (a)
  begin
    y_incomplete <= a or b;
  end process;

  clk_gen : process
  begin
    while not done loop
      clk <= '0';
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
    end loop;
    wait;
  end process;

  counter : process (clk)
  begin
    if rising_edge(clk) then
      cnt <= cnt + 1;
    end if;
  end process;

  waiter : process
  begin
    wait on b;
    ev <= ev + 1;
    wait until b = '0';
    ev <= ev + 10;
    wait;
  end process;

  stim : process
  begin
    wait for 1 ns;
    a <= '1';
    wait for 1 ns;
    if y_list = '1' and y_all = '0' then report "sens list ok"; else report "sens list bad"; end if;
    if y_incomplete = '1' then report "incomplete 1 ok"; else report "incomplete 1 bad"; end if;
    a <= '0';
    wait for 1 ns;
    b <= '1';
    wait for 1 ns;
    if y_list = '1' and y_all = '0' then report "b change ok"; else report "b change bad"; end if;
    if y_incomplete = '0' then report "incomplete held ok"; else report "incomplete held bad"; end if;
    if ev = 1 then report "wait on ok"; else report "wait on bad"; end if;
    b <= '0';
    wait for 1 ns;
    if ev = 11 then report "wait until ok"; else report "wait until bad"; end if;
    wait until cnt = 3;
    wait until clk = '1' for 100 ns;
    wait on clk for 1 ns;
    done <= true;
    wait for 20 ns;
    if cnt = 5 then report "clock stopped ok"; else report "clock stopped bad"; end if;
    wait;
  end process;
end architecture;
