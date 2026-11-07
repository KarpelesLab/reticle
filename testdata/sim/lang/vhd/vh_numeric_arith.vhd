library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
begin
  process
    variable ua : unsigned(7 downto 0) := to_unsigned(200, 8);
    variable ub : unsigned(7 downto 0) := to_unsigned(13, 8);
    variable sa : signed(7 downto 0) := to_signed(-100, 8);
    variable sb : signed(7 downto 0) := to_signed(7, 8);
    variable u16 : unsigned(15 downto 0);
    variable s16 : signed(15 downto 0);
    variable u4 : unsigned(3 downto 0);
  begin
    report "ua+ub=" & integer'image(to_integer(ua + ub));
    report "ua-ub=" & integer'image(to_integer(ua - ub));
    report "ub-ua=" & integer'image(to_integer(ub - ua));
    u16 := ua * ub;
    report "ua*ub=" & integer'image(to_integer(u16));
    report "ua/ub=" & integer'image(to_integer(ua / ub));
    report "ua mod ub=" & integer'image(to_integer(ua mod ub));
    report "ua rem ub=" & integer'image(to_integer(ua rem ub));
    report "ua+1=" & integer'image(to_integer(ua + 1));
    report "ua+60 (wrap)=" & integer'image(to_integer(ua + 60));
    report "sa+sb=" & integer'image(to_integer(sa + sb));
    report "sa-sb=" & integer'image(to_integer(sa - sb));
    s16 := sa * sb;
    report "sa*sb=" & integer'image(to_integer(s16));
    report "sa/sb=" & integer'image(to_integer(sa / sb));
    report "sa mod sb=" & integer'image(to_integer(sa mod sb));
    report "sa rem sb=" & integer'image(to_integer(sa rem sb));
    report "sb mod -3=" & integer'image(to_integer(sb mod to_signed(-3, 8)));
    report "sb rem -3=" & integer'image(to_integer(sb rem to_signed(-3, 8)));
    report "abs sa=" & integer'image(to_integer(abs sa));
    report "-sa=" & integer'image(to_integer(-sa));
    report "abs -128=" & to_hstring(abs to_signed(-128, 8));
    report "sa-1=" & integer'image(to_integer(sa - 1));
    report "sa*2 int=" & integer'image(to_integer(sa * 2));
    report "resize u 4=" & to_string(resize(ua, 4));
    report "resize s 4=" & to_string(resize(sa, 4));
    report "resize s 12=" & to_string(resize(sa, 12));
    report "resize u 12=" & to_string(resize(ua, 12));
    report "to_unsigned(5,3)=" & to_string(to_unsigned(5, 3));
    report "to_signed(-5,4)=" & to_string(to_signed(-5, 4));
    report "signed(slv)=" & integer'image(to_integer(signed(std_logic_vector'(x"F0"))));
    report "unsigned(slv)=" & integer'image(to_integer(unsigned(std_logic_vector'(x"F0"))));
    report "slv(u)=" & to_hstring(std_logic_vector(ua));
    u4 := ua(3 downto 0) + ub(3 downto 0);
    report "u4=" & integer'image(to_integer(u4));
    report "ua*ub length=" & integer'image(u16'length);
    report "ua > ub " & boolean'image(ua > ub);
    report "u8 = u4 diff width " & boolean'image(to_unsigned(5, 8) = to_unsigned(5, 4));
    report "u > int " & boolean'image(ua > 199);
    report "s < int " & boolean'image(sa < -99);
    report "s vs s diff width " & boolean'image(to_signed(-1, 4) = to_signed(-1, 8));
    report "max min " & integer'image(to_integer(maximum(ua, ub))) & " " & integer'image(to_integer(minimum(sa, sb)));
    report "std_match " & boolean'image(std_match(unsigned'("1010"), unsigned'("1-1-")));
    report "find leftmost " & integer'image(find_leftmost(to_unsigned(20, 8), '1'));
    report "find rightmost " & integer'image(find_rightmost(to_unsigned(20, 8), '1'));
    wait;
  end process;
end architecture;
