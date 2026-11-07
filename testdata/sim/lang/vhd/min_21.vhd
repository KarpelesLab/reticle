library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  type pt is record x, y : integer; end record;
  type outer is record p : pt; n : integer; end record;
begin
  process
    variable o : outer := ((1, 2), 3);
  begin
    o.p.x := 7;
    if o.p.x = 7 and o.p.y = 2 then report "nested target ok"; else report "nested target bad"; end if;
    wait;
  end process;
end architecture;
