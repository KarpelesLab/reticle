library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  subtype small_t is integer range -10 to 10;
  signal n : natural := 5;
  signal p : positive := 1;
begin
  process
    variable a : integer := -7;
    variable b : integer := 3;
    variable s : small_t := 0;
  begin
    report "7 mod 3=" & integer'image(7 mod 3);
    report "-7 mod 3=" & integer'image(a mod b);
    report "7 mod -3=" & integer'image(7 mod (-3));
    report "-7 mod -3=" & integer'image(a mod (-b));
    report "7 rem 3=" & integer'image(7 rem 3);
    report "-7 rem 3=" & integer'image(a rem b);
    report "7 rem -3=" & integer'image(7 rem (-3));
    report "-7 rem -3=" & integer'image(a rem (-b));
    report "-7/3=" & integer'image(a / b);
    report "7/-3=" & integer'image(7 / (-3));
    report "2**10=" & integer'image(2 ** 10);
    report "3**0=" & integer'image(3 ** 0);
    report "abs -7=" & integer'image(abs a);
    report "high=" & integer'image(integer'high);
    report "low=" & integer'image(integer'low);
    report "natural'low=" & integer'image(natural'low);
    report "positive'low=" & integer'image(positive'low);
    report "small'high=" & integer'image(small_t'high);
    report "small'low=" & integer'image(small_t'low);
    report "small'left=" & integer'image(small_t'left);
    report "s init=" & integer'image(s);
    report "integer'value=" & integer'image(integer'value("  -42"));
    report "1_000=" & integer'image(1_000);
    report "16#FF#=" & integer'image(16#FF#);
    report "2#1010#=" & integer'image(2#1010#);
    report "8#17#=" & integer'image(8#17#);
    report "1e3=" & integer'image(1e3);
    report "n+p=" & integer'image(n + p);
    report "mixed=" & integer'image((a * b + 100) / 4 - 2 ** 3 mod 5);
    report "max int sum=" & integer'image(integer'high - 1 + 1);
    report "pos(5)=" & integer'image(integer'pos(5));
    report "succ(5)=" & integer'image(integer'succ(5));
    report "pred(-5)=" & integer'image(integer'pred(-5));
    wait;
  end process;
end architecture;
