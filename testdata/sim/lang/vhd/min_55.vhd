library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal x : std_logic := 'X';
  signal u : std_logic := 'U';
begin
  process
  begin
    if ?? 'X' then report "?? X true (wrong)"; else report "?? X false ok"; end if;
    if ?? std_logic'('U') then report "?? U true (wrong)"; else report "?? U false ok"; end if;
    if x = '1' then report "X = 1 true (wrong)"; else report "X = 1 false ok"; end if;
    wait;
  end process;
end architecture;
