library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  constant TABLE : integer := 5;
  alias T is TABLE;
begin
  process
  begin
    if T = 5 then report "constant alias ok"; else report "constant alias bad"; end if;
    wait;
  end process;
end architecture;
