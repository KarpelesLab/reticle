library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable a : std_logic_vector(3 downto 0) := "1000";
    variable b : std_logic_vector(3 downto 0);
  begin
    b := a rol 1;
    if b = "0001" then report "rol ok"; else report "rol bad"; end if;
    wait;
  end process;
end architecture;
