library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable a : std_logic_vector(3 downto 0) := "1100";
  begin
    if (a & a) = x"CC" then report "concat eq ok"; else report "concat eq bad"; end if;
    wait;
  end process;
end architecture;
