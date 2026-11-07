library ieee;
use ieee.std_logic_1164.all;

entity inv is
  port (a : in std_logic; y : out std_logic);
end entity;

architecture good of inv is
begin
  y <= not a;
end architecture;

architecture buf of inv is
begin
  y <= a;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  component inv
    port (a : in std_logic; y : out std_logic);
  end component;
  for u1 : inv use entity work.inv(buf);
  signal a, y1, y2, y3 : std_logic := '0';
begin
  u1 : inv port map (a, y1);
  u2 : inv port map (a, y2);
  u3 : entity work.inv(buf) port map (a, y3);
  process
  begin
    a <= '1';
    wait for 1 ns;
    if y1 = '1' then report "config spec buf ok"; else report "config spec buf bad"; end if;
    if y2 = '0' then report "default arch (last) ok"; else report "default arch is not last-analysed"; end if;
    if y3 = '1' then report "entity(arch) ok"; else report "entity(arch) bad"; end if;
    wait;
  end process;
end architecture;
