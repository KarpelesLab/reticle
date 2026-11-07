library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal a : std_logic_vector(7 downto 0) := 8x"F";
  signal b : std_logic_vector(7 downto 0) := 8sx"F";
  signal c : std_logic_vector(5 downto 0) := o"17";
  signal d : std_logic_vector(2 downto 0) := b"1_0_1";
  signal e : std_logic_vector(9 downto 0) := 10d"513";
  signal f : std_logic_vector(7 downto 0) := 8b"11";
  signal g : std_logic_vector(3 downto 0) := x"-";
  signal h : std_logic_vector(7 downto 0) := 8ux"A";
begin
  process
  begin
    if a = "00001111" then report "8x ok"; else report "8x bad"; end if;
    if b = "11111111" then report "8sx ok"; else report "8sx bad"; end if;
    if c = "001111" then report "octal ok"; else report "octal bad"; end if;
    if d = "101" then report "underscore ok"; else report "underscore bad"; end if;
    if e = "1000000001" then report "decimal ok"; else report "decimal bad"; end if;
    if f = "00000011" then report "8b ok"; else report "8b bad"; end if;
    if g = "----" then report "x dash ok"; else report "x dash bad"; end if;
    if h = "00001010" then report "8ux ok"; else report "8ux bad"; end if;
    wait;
  end process;
end architecture;
