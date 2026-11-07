library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal s : std_logic := '0';
  signal n : integer := 7;
  signal saw_event : boolean := false;
  signal saw_rise, saw_fall : integer := 0;
  signal dly : std_logic;
  signal st : boolean;
begin
  watch : process (s)
  begin
    if s'event and s = '1' then
      saw_rise <= saw_rise + 1;
    end if;
    if falling_edge(s) then
      saw_fall <= saw_fall + 1;
    end if;
  end process;

  dly <= s'delayed(2 ns);
  st <= s'stable(3 ns);

  process
  begin
    wait for 1 ns;
    s <= '1';
    n <= 9;
    wait for 0 ns;
    if s'event then report "event ok"; else report "event bad"; end if;
    if s'last_value = '0' then report "last_value ok"; else report "last_value bad"; end if;
    if n'last_value = 7 then report "int last_value ok"; else report "int last_value bad"; end if;
    if s'active then report "active ok"; else report "active bad"; end if;
    wait for 1 ns;
    if not s'event then report "no event ok"; else report "no event bad"; end if;
    if s'last_event = 1 ns then report "last_event ok"; else report "last_event bad"; end if;
    if saw_rise = 1 then report "rise seen ok"; else report "rise seen bad"; end if;
    if dly = '0' then report "delayed before ok"; else report "delayed before bad"; end if;
    if st = false then report "stable false ok"; else report "stable false bad"; end if;
    wait for 2 ns;
    if dly = '1' then report "delayed after ok"; else report "delayed after bad"; end if;
    if st = true then report "stable true ok"; else report "stable true bad"; end if;
    s <= '0';
    wait for 1 ns;
    if saw_fall = 1 then report "fall seen ok"; else report "fall seen bad"; end if;
    if s'stable then report "stable now ok"; else report "stable now bad"; end if;
    wait;
  end process;
end architecture;
