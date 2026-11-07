library ieee;
use ieee.std_logic_1164.all;

entity sub is
  generic (NAME : string := "unit");
  port (nl : out integer);
end entity;

architecture rtl of sub is
begin
  nl <= NAME'length;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal n : integer;
begin
  u : entity work.sub generic map (NAME => "second") port map (nl => n);
  process
  begin
    wait for 1 ns;
    if n = 6 then report "string generic ok"; else report "string generic bad"; end if;
    wait;
  end process;
end architecture;
