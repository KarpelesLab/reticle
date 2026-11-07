library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  type rec is record a : integer; b : std_logic; end record;
  type rec_arr is array (0 to 1) of rec;
  signal r : rec_arr := (others => (0, '0'));
begin
  process
  begin
    r(1).a <= 5;
    wait for 1 ns;
    if r(1).a = 5 and r(0).a = 0 then report "arr rec field ok"; else report "arr rec field bad"; end if;
    wait;
  end process;
end architecture;
