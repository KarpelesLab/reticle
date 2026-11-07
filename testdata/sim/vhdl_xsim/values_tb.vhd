-- Values that came out wrong, held to what Vivado 2026.1's xsim printed for
-- this file (values_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is end entity;

architecture sim of tb is
  type down_arr is array (7 downto 4) of integer;
  type rom_t is array (3 downto 0) of std_logic_vector(7 downto 0);
  constant ROM : rom_t := (x"A0", x"B1", x"C2", x"D3");
  constant TABLE : integer := 5;
  alias T is TABLE;
  signal w : std_logic_vector(7 downto 0) := x"80";
  alias rev : std_logic_vector(0 to 7) is w(7 downto 0);
  signal a, b : std_logic;
  signal addr : integer range 0 to 3 := 0;
  function classify(n : integer) return integer is
  begin
    case n is
      when integer'low to -1 => return 1;
      when 0 => return 2;
      when others => return 3;
    end case;
  end function;
begin
  process
    variable d : down_arr := (1, 2, 3, 4);
    variable p, q : std_logic;
    variable n : integer := -7;
  begin
    report "d(7)=" & integer'image(d(7)) & " d(4)=" & integer'image(d(4));
    for i in 0 to 3 loop
      addr <= i;
      wait for 1 ns;
      report "ROM(" & integer'image(i) & ")=" & to_hstring(ROM(addr));
    end loop;
    (p, q) := std_logic_vector'("01");
    (a, b) <= std_logic_vector'("10");
    wait for 1 ns;
    report "p=" & std_logic'image(p) & " q=" & std_logic'image(q)
      & " a=" & std_logic'image(a) & " b=" & std_logic'image(b);
    report "T=" & integer'image(T) & " rev(0)=" & std_logic'image(rev(0))
      & " rev(7)=" & std_logic'image(rev(7));
    report "classify " & integer'image(classify(-5)) & integer'image(classify(0))
      & integer'image(classify(9)) & integer'image(classify(n));
    case n is
      when integer'low to -1 => report "negative";
      when others => report "not negative";
    end case;
    wait;
  end process;
end architecture;
