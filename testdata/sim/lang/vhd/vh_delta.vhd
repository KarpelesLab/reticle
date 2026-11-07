library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal s : integer := 0;
  signal s1, s2, s3 : integer := 0;
  signal x : std_logic := '0';
  signal deltas : integer := 0;
begin
  s1 <= s + 1;
  s2 <= s1 + 1;
  s3 <= s2 + 1;

  dcount : process (x)
  begin
    deltas <= deltas + 1;
  end process;

  process
    variable v : integer := 0;
  begin
    -- variable updates are immediate; signal updates take a delta
    v := 5;
    s <= 5;
    if v = 5 then report "var immediate ok"; else report "var immediate bad"; end if;
    if s = 0 then report "sig deferred ok"; else report "sig deferred bad"; end if;
    wait for 0 ns;
    if s = 5 then report "after delta ok"; else report "after delta bad"; end if;
    if s1 = 1 then report "s1 lags ok"; else report "s1 lags bad"; end if;
    wait for 0 ns;
    if s1 = 6 and s2 = 2 then report "chain delta 2 ok"; else report "chain delta 2 bad"; end if;
    wait for 0 ns;
    if s3 = 8 then report "chain delta 3 ok"; else report "chain delta 3 bad"; end if;
    wait for 1 ns;
    -- last assignment in a process wins
    s <= 1;
    s <= 2;
    s <= 3;
    wait for 0 ns;
    if s = 3 then report "last wins ok"; else report "last wins bad"; end if;
    wait for 1 ns;
    x <= '1';
    wait for 0 ns;
    x <= '0';
    wait for 0 ns;
    x <= '1';
    wait for 1 ns;
    if deltas = 4 then report "delta events ok"; else report "delta events bad"; end if;
    wait;
  end process;
end architecture;
