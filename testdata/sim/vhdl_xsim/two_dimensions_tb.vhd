-- Arrays of two dimensions, held to what Vivado 2026.1's xsim printed for
-- this file (two_dimensions_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;

entity tb is end entity;

architecture sim of tb is
  type mat_t is array (0 to 2, 0 to 3) of integer;
  type bits_t is array (1 downto 0, 0 to 3) of std_logic;
  type rev_t is array (3 downto 1, 2 to 3) of integer;
  constant TABLE : mat_t := ((0, 1, 2, 3), (10, 11, 12, 13), (20, 21, 22, 23));
  signal grid : bits_t := (('1', '0', '0', '0'), ('0', '0', '0', '1'));
begin
  process
    variable m : mat_t := (others => (others => 0));
    variable r : rev_t := ((1, 2), (3, 4), (5, 6));
    variable i, j : integer;
  begin
    m(1, 3) := 13;
    report "m(1,3)=" & integer'image(m(1, 3)) & " m(0,0)=" & integer'image(m(0, 0));
    for a in 0 to 2 loop
      for b in 0 to 3 loop
        m(a, b) := a * 10 + b;
      end loop;
    end loop;
    i := 2;
    j := 3;
    report "m(i,j)=" & integer'image(m(i, j)) & " m(1,0)=" & integer'image(m(1, 0))
      & " TABLE(2,1)=" & integer'image(TABLE(2, 1)) & " TABLE(i,0)=" & integer'image(TABLE(i, 0));
    report "lengths " & integer'image(m'length(1)) & integer'image(m'length(2))
      & " high(2)=" & integer'image(m'high(2));
    report "r(3,2)=" & integer'image(r(3, 2)) & " r(1,3)=" & integer'image(r(1, 3));
    grid(0, 2) <= '1';
    wait for 1 ns;
    report "grid(1,0)=" & std_logic'image(grid(1, 0)) & " grid(0,3)=" & std_logic'image(grid(0, 3))
      & " grid(0,2)=" & std_logic'image(grid(0, 2)) & " grid(1,j)=" & std_logic'image(grid(1, j));
    wait;
  end process;
end architecture;
