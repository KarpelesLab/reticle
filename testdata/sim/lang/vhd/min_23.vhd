library ieee;
use ieee.std_logic_1164.all;

entity sub is
  generic (INIT : std_logic_vector := x"0F");
  port (iv : out std_logic_vector(INIT'range));
end entity;

architecture rtl of sub is
begin
  iv <= INIT;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal i2 : std_logic_vector(11 downto 0);
begin
  u : entity work.sub generic map (INIT => x"ABC") port map (iv => i2);
  process
  begin
    wait for 1 ns;
    if i2 = x"ABC" then report "unconstrained generic ok"; else report "unconstrained generic bad"; end if;
    wait;
  end process;
end architecture;
