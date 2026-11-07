library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std_unsigned.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable a : std_logic_vector(7 downto 0) := x"30";
  begin
    a := a + 1;
    if a = x"31" then report "nsu ok"; else report "nsu bad"; end if;
    wait;
  end process;
end architecture;
