library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std_unsigned.all;

entity tb is
end entity;

architecture sim of tb is
  signal cnt : std_logic_vector(7 downto 0) := x"FE";
begin
  process
    variable a : std_logic_vector(7 downto 0) := x"30";
    variable b : std_logic_vector(7 downto 0) := x"05";
    variable p : std_logic_vector(15 downto 0);
  begin
    report "a+b=" & to_hstring(a + b);
    report "a-b=" & to_hstring(a - b);
    report "b-a=" & to_hstring(b - a);
    p := a * b;
    report "a*b=" & to_hstring(p);
    report "a/b=" & to_hstring(a / b);
    report "a mod 7=" & to_hstring(a mod 7);
    report "a+1=" & to_hstring(a + 1);
    report "to_integer(a)=" & integer'image(to_integer(a));
    report "to_slv(77,8)=" & to_hstring(to_slv(77, 8));
    report "to_stdlogicvector(9,4)=" & to_string(to_stdlogicvector(9, 4));
    report "a > b " & boolean'image(a > b);
    report "a < 100 " & boolean'image(a < 100);
    report "x0F vs x00F " & boolean'image(std_logic_vector'(x"0F") = std_logic_vector'(x"00F"));
    report "shift_left(a,2)=" & to_hstring(shift_left(a, 2));
    report "resize(a,4)=" & to_string(resize(a, 4));
    cnt <= cnt + 1;
    wait for 1 ns;
    report "cnt=" & to_hstring(cnt);
    cnt <= cnt + 1;
    wait for 1 ns;
    report "cnt wrap=" & to_hstring(cnt);
    if cnt = 0 then report "cnt is zero"; end if;
    wait;
  end process;
end architecture;
