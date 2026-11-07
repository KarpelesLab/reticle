library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal src : std_logic := '0';
  signal inert, trans, rej : std_logic := '0';
  signal wave : integer := 0;
  signal tr2 : integer := 0;
begin
  inert <= src after 3 ns;
  trans <= transport src after 3 ns;
  rej <= reject 1 ns inertial src after 3 ns;

  process
  begin
    -- 2 ns pulse at t=1: inertial (3 ns) swallows it, transport passes it
    wait for 1 ns;
    src <= '1';
    wait for 2 ns;
    src <= '0';
    wait for 1 ns;   -- t=4
    if trans = '1' then report "transport passes pulse ok"; else report "transport passes pulse bad"; end if;
    if inert = '0' then report "inertial swallows ok"; else report "inertial swallows bad"; end if;
    if rej = '1' then report "reject 1ns passes 2ns pulse ok"; else report "reject 1ns passes 2ns pulse bad"; end if;
    wait for 2 ns;   -- t=6
    if trans = '0' then report "transport falls ok"; else report "transport falls bad"; end if;
    -- long pulse passes both
    src <= '1';
    wait for 5 ns;   -- t=11
    if inert = '1' and trans = '1' then report "long pulse ok"; else report "long pulse bad"; end if;
    src <= '0';
    -- waveform with multiple elements
    wave <= 1 after 1 ns, 2 after 2 ns, 3 after 4 ns;
    wait for 1500 ps;
    if wave = 1 then report "wave 1 ok"; else report "wave 1 bad"; end if;
    wait for 1 ns;
    if wave = 2 then report "wave 2 ok"; else report "wave 2 bad"; end if;
    wait for 2 ns;
    if wave = 3 then report "wave 3 ok"; else report "wave 3 bad"; end if;
    -- a new inertial assignment cancels pending transactions
    wave <= 10 after 2 ns;
    wait for 1 ns;
    wave <= 20 after 3 ns;
    wait for 1500 ps;
    if wave = 3 then report "inertial cancel ok"; else report "inertial cancel bad"; end if;
    wait for 2 ns;
    if wave = 20 then report "inertial final ok"; else report "inertial final bad"; end if;
    -- transport keeps earlier transactions
    tr2 <= transport 1 after 2 ns;
    tr2 <= transport 2 after 4 ns;
    wait for 3 ns;
    if tr2 = 1 then report "transport keep ok"; else report "transport keep bad"; end if;
    wait for 2 ns;
    if tr2 = 2 then report "transport second ok"; else report "transport second bad"; end if;
    wait;
  end process;
end architecture;
