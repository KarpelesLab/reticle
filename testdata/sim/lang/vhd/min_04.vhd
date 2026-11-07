library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable u : unsigned(7 downto 0) := x"A6";
  begin
    if u = x"A6" then report "eq hex ok"; else report "eq hex bad"; end if;
    if u = "10100110" then report "eq str ok"; else report "eq str bad"; end if;
    if signed(u) = "10100110" then report "eq sig ok"; else report "eq sig bad"; end if;
    wait;
  end process;
end architecture;
