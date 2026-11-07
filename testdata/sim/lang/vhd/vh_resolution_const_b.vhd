library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal a, b, c, d, e, f, g, h : std_logic;
  signal bus8 : std_logic_vector(3 downto 0);
begin
  a <= '0'; a <= '1';
  b <= '1'; b <= 'Z';
  c <= 'H'; c <= 'L';
  d <= 'L'; d <= 'Z';
  e <= 'H'; e <= '0';
  f <= 'W'; f <= 'H';
  g <= 'Z'; g <= 'Z';
  h <= 'U'; h <= '1';
  bus8 <= "10ZH";
  bus8 <= "Z0LZ";
  chk : process
  begin
    wait for 1 ns;
    if a = 'X' then report "a ok"; else report "a bad"; end if;
    if b = '1' then report "b ok"; else report "b bad"; end if;
    if c = 'W' then report "c ok"; else report "c bad"; end if;
    if d = 'L' then report "d ok"; else report "d bad"; end if;
    if e = '0' then report "e ok"; else report "e bad"; end if;
    if f = 'W' then report "f ok"; else report "f bad"; end if;
    if g = 'Z' then report "g ok"; else report "g bad"; end if;
    if h = 'U' then report "h ok"; else report "h bad"; end if;
    if bus8 = "100H" then report "bus ok"; else report "bus bad"; end if;
    if bus8(0) = 'H' then report "bus0 H"; end if;
    if bus8(1) = '0' then report "bus1 0"; end if;
    if bus8(2) = '0' then report "bus2 0"; end if;
    if bus8(3) = '1' then report "bus3 1"; end if;
    if to_x01(c) = 'X' then report "to_x01 W ok"; end if;
    if to_x01(d) = '0' then report "to_x01 L ok"; end if;
    if ('H' and '1') = '1' then report "H and 1 ok"; end if;
    if ('L' or 'Z') = 'X' then report "L or Z X ok"; else report "L or Z not X"; end if;
    if (not 'Z') = 'X' then report "not Z ok"; end if;
    if (not 'U') = 'U' then report "not U ok"; end if;
    wait;
  end process;
end architecture;
