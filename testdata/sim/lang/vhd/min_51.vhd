library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
    variable bv : bit_vector(3 downto 0);
  begin
    bv := to_bitvector(std_logic_vector'("1H0L"));
    if bv = "1100" then report "to_bitvector ok"; else report "to_bitvector bad"; end if;
    wait;
  end process;
end architecture;
