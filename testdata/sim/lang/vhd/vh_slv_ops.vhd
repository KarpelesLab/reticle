library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  signal a : std_logic_vector(7 downto 0) := x"C3";
  signal b : std_logic_vector(7 downto 0) := "01011010";
  signal r : std_logic_vector(7 downto 0);
  signal w : std_logic_vector(15 downto 0);
begin
  process
    variable v : std_logic_vector(7 downto 0);
    variable u : std_logic_vector(0 to 7);
  begin
    report "a and b = " & to_hstring(a and b);
    report "a or b = " & to_hstring(a or b);
    report "a xor b = " & to_hstring(a xor b);
    report "a nand b = " & to_hstring(a nand b);
    report "a nor b = " & to_hstring(a nor b);
    report "a xnor b = " & to_hstring(a xnor b);
    report "not a = " & to_hstring(not a);
    report "a & b = " & to_hstring(a & b);
    report "a(3 downto 0) & '1' = " & to_string(a(3 downto 0) & '1');
    report "slice a(6 downto 2) = " & to_string(a(6 downto 2));
    report "a sll 2 = " & to_string(a sll 2);
    report "a srl 3 = " & to_string(a srl 3);
    report "a rol 1 = " & to_string(a rol 1);
    report "a ror 2 = " & to_string(a ror 2);
    report "a sll -1 = " & to_string(a sll -1);
    report "and '1' scalar = " & to_string(a and '1');
    report "'0' or b = " & to_string('0' or b);
    v := a;
    v(7 downto 4) := b(3 downto 0);
    report "v = " & to_hstring(v);
    u := v;
    report "u(0) = " & std_logic'image(u(0)) & " u(7) = " & std_logic'image(u(7));
    report "u(0 to 3) = " & to_string(u(0 to 3));
    r <= a xor x"FF";
    w <= a & b;
    wait for 1 ns;
    report "r = " & to_hstring(r);
    report "w = " & to_hstring(w);
    report "a = b " & boolean'image(a = b);
    report "a /= b " & boolean'image(a /= b);
    report "a > b " & boolean'image(a > b);
    report "a < b " & boolean'image(a < b);
    report "a >= x80 " & boolean'image(a >= x"80");
    report "short < long " & boolean'image(std_logic_vector'("11") < std_logic_vector'("100"));
    report "eq diff len " & boolean'image(std_logic_vector'("01") = std_logic_vector'("001"));
    report "x string " & to_string(std_logic_vector'("UX01ZWLH-"));
    report "o17 " & to_string(std_logic_vector'(o"17"));
    report "b101 " & to_string(std_logic_vector'(b"1_0_1"));
    wait;
  end process;
end architecture;
