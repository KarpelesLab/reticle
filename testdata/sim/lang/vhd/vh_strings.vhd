library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  constant GREETING : string := "hello";
  constant S2 : string(1 to 3) := "abc";
  type char_arr is array (1 to 4) of character;
begin
  process
    variable s : string(1 to 5);
    variable c : character;
    variable ca : char_arr := ('w', 'x', 'y', 'z');
    variable u : unsigned(11 downto 0) := x"A5C";
    variable v : std_logic_vector(7 downto 0) := "1100ZX01";
  begin
    report GREETING & ", world";
    report "len=" & integer'image(GREETING'length);
    report "char 2=" & GREETING(2);
    report "slice=" & GREETING(2 to 4);
    s := GREETING;
    s(1) := 'J';
    report "modified=" & s;
    report "S2 reversed? " & S2(3) & S2(2) & S2(1);
    c := 'Q';
    report "char image=" & character'image(c);
    report "int image=" & integer'image(-42);
    report "bool image=" & boolean'image(GREETING = "hello");
    report "eq=" & boolean'image(GREETING = "hellp");
    report "lt=" & boolean'image(string'("abc") < string'("abd"));
    report "to_string int=" & to_string(123);
    report "to_string bool=" & to_string(false);
    report "to_string char=" & to_string('k');
    report "to_hstring u=" & to_hstring(u);
    report "to_ostring u=" & to_ostring(u);
    report "to_string u=" & to_string(u);
    report "to_string v=" & to_string(v);
    report "to_hstring v=" & to_hstring(v);
    report "to_hstring 5 bits=" & to_hstring(std_logic_vector'("10101"));
    report "to_ostring 4 bits=" & to_ostring(std_logic_vector'("1111"));
    report "signed hstring=" & to_hstring(to_signed(-3, 8));
    report "time string=" & to_string(15 ns);
    report "real string=" & to_string(1.5);
    report "character array=" & ca(1) & ca(4);
    report "integer'image(16#7F#)=" & integer'image(16#7F#);
    report "nested image: " & integer'image(integer'value("77") + 1);
    wait;
  end process;
end architecture;
