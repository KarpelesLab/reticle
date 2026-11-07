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
    r := v(5 downto 0) & '0' & '0';
    if r(7) = '0' and r(6) = '0' and r(2) = '1' then report "concat chr ok"; else report "concat chr bad"; end if;
    r := v sll 2;
    if r(2) = '1' then report "sll bit2 ok"; else report "sll bit2 bad"; end if;
    r := v srl 2;
    if r(5) = '1' then report "srl bit5 ok"; else report "srl bit5 bad"; end if;
    wait;
  end process;
end architecture;
