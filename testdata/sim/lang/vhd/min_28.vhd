library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal a, b : std_logic;
begin
  process
    variable p, q : std_logic;
  begin
    (p, q) := std_logic_vector'("01");
    if p = '0' and q = '1' then report "var target ok"; elsif p = '1' and q = '0' then report "var target reversed"; else report "var target other"; end if;
    (a, b) <= std_logic_vector'("10");
    wait for 1 ns;
    if a = '1' and b = '0' then report "sig target ok"; elsif a = '0' and b = '1' then report "sig target reversed"; else report "sig target other"; end if;
    wait;
  end process;
end architecture;
