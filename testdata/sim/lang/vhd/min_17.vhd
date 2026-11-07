library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable v : std_logic_vector(7 downto 0) := x"C3";
    variable r : std_logic_vector(7 downto 0);
  begin
    r := v(5 downto 0) & "00";
    if r(7) = '0' and r(6) = '0' and r(2) = '1' then report "concat lit ok"; else report "concat lit bad"; end if;
    wait;
  end process;
end architecture;
