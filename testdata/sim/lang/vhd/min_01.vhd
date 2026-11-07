library ieee;
use ieee.std_logic_1164.all;
entity tb is end entity;
architecture sim of tb is
  signal s00, s01, sx, sh, sw, szh : std_logic;
  signal su : std_logic := 'U';
begin
  s00 <= '0'; s00 <= '0';
  s01 <= '0'; s01 <= '1';
  sx <= 'X';
  sh <= 'H';
  sw <= 'W';
  szh <= 'Z'; szh <= 'H';
  process begin
    wait for 1 ns;
    if s00 = '0' then report "s00 is 0"; else report "s00 not 0"; end if;
    if s01 = 'X' then report "s01 is X"; else report "s01 not X"; end if;
    if s01 = 'U' then report "s01 is U"; else report "s01 not U"; end if;
    if sx = 'X' then report "sx is X"; else report "sx not X"; end if;
    if su = 'X' then report "su is X"; else report "su not X"; end if;
    if sh = 'H' then report "sh is H"; else report "sh not H"; end if;
    if sh = '1' then report "sh is 1"; else report "sh not 1"; end if;
    if sw = 'W' then report "sw is W"; else report "sw not W"; end if;
    if szh = 'H' then report "szh is H"; else report "szh not H"; end if;
    wait;
  end process;
end architecture;
