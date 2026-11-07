library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal a : std_logic_vector(7 downto 0) := x"C3";
  signal b : std_logic_vector(7 downto 0) := "01011010";
  signal r : std_logic_vector(7 downto 0);
  signal w : std_logic_vector(15 downto 0);
  constant K55 : std_logic_vector(7 downto 0) := x"55";
begin
  process
    variable v : std_logic_vector(7 downto 0);
    variable u : std_logic_vector(0 to 7);
  begin
    if (a and b) = x"42" then report "and ok"; else report "and bad"; end if;
    if (a or b) = x"DB" then report "or ok"; else report "or bad"; end if;
    if (a xor b) = x"99" then report "xor ok"; else report "xor bad"; end if;
    if (a nand b) = x"BD" then report "nand ok"; else report "nand bad"; end if;
    if (a nor b) = x"24" then report "nor ok"; else report "nor bad"; end if;
    if (a xnor b) = x"66" then report "xnor ok"; else report "xnor bad"; end if;
    if (not a) = x"3C" then report "not ok"; else report "not bad"; end if;
    if (a & b) = x"C35A" then report "concat ok"; else report "concat bad"; end if;
    if a(6 downto 2) = "10000" then report "slice ok"; else report "slice bad"; end if;
    if (a sll 2) = "00001100" then report "sll ok"; else report "sll bad"; end if;
    if (a srl 3) = "00011000" then report "srl ok"; else report "srl bad"; end if;
    if (a sll -1) = "01100001" then report "sll neg ok"; else report "sll neg bad"; end if;
    if (a and '1') = a then report "scalar and ok"; else report "scalar and bad"; end if;
    v := a;
    v(7 downto 4) := b(3 downto 0);
    if v = x"A3" then report "partial ok"; else report "partial bad"; end if;
    u := v;
    if u(0) = '1' and u(7) = '1' and u(1) = '0' then report "asc ok"; else report "asc bad"; end if;
    if u(0 to 3) = "1010" then report "asc slice ok"; else report "asc slice bad"; end if;
    r <= a xor x"FF";
    w <= a & b;
    wait for 1 ns;
    if r = x"3C" then report "sig r ok"; else report "sig r bad"; end if;
    if w = x"C35A" then report "sig w ok"; else report "sig w bad"; end if;
    if a /= b then report "ne ok"; else report "ne bad"; end if;
    if a > b then report "gt ok"; else report "gt bad"; end if;
    if std_logic_vector'("11") < std_logic_vector'("100") then report "lex lt bad"; else report "lex lt ok"; end if;
    if std_logic_vector'("01") = std_logic_vector'("001") then report "eq len bad"; else report "eq len ok"; end if;
    if (K55 or x"AA") = x"FF" then report "const ok"; else report "const bad"; end if;
    if (a rol 1) = "10000111" then report "rol ok"; else report "rol bad"; end if;
    if (a ror 2) = "11110000" then report "ror ok"; else report "ror bad"; end if;
    wait;
  end process;
end architecture;
