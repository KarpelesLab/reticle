library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    type c_t is (r, g, b);
    variable c : c_t := g;
  begin
    if c_t'succ(c) = b then report "succ ok"; else report "succ bad"; end if;
    if c_t'pos(c) = 1 then report "pos ok"; else report "pos bad"; end if;
    if c_t'image(c) = "g" then report "image ok"; else report "image bad"; end if;
    wait;
  end process;
end architecture;
