-- Report messages built at run time, held to what Vivado 2026.1's xsim
-- printed for this file (report_strings_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is end entity;

architecture sim of tb is
  type color_t is (red, green, blue);
  signal level : integer := 3;
begin
  process
    variable i : integer := 5;
    variable n : integer := -42;
    variable b : boolean := true;
    variable t : time := 3 ns;
    variable ch : character := 'Q';
    variable bv : bit_vector(3 downto 0) := "1010";
    variable sl : std_logic := '1';
    variable u : unsigned(7 downto 0) := x"A6";
    variable s : signed(5 downto 0) := "101010";
    variable v : std_logic_vector(7 downto 0) := x"3C";
    variable c : color_t := green;
  begin
    report integer'image(i);
    report "i=" & integer'image(i) & ", n=" & integer'image(n);
    report "b=" & boolean'image(b) & " t=" & time'image(t);
    report "ch=" & character'image(ch) & " sl=" & std_logic'image(sl);
    report character'image('A');
    report "c=" & color_t'image(c) & " succ=" & color_t'image(color_t'succ(c));
    report "pos=" & integer'image(color_t'pos(c)) & " val=" & color_t'image(color_t'val(i - 3));
    report "pred=" & integer'image(integer'pred(i));
    -- xsim writes `g` for color_t'image(maximum(c, red)), one character of
    -- `green`, which looks like a defect of its own; the comparison is
    -- what is held here.
    if maximum(c, red) = green and minimum(blue, c) = green then report "extremes"; end if;
    report to_string(i) & " " & to_string(b) & " " & to_string(t) & " " & to_string(ch);
    report to_string(c) & " " & to_string(bv) & " " & to_hstring(bv) & " " & to_string(sl);
    report "u=" & to_string(u) & " h=" & to_hstring(u) & " o=" & to_ostring(u);
    report "s=" & to_hstring(s) & " v=" & to_hstring(v) & " " & to_string(v);
    report "a" & "b" & "c";
    report "x" & 'y' & "z" & ch;
    report "100% sure, " & integer'image(level) & "%";
    if color_t'image(c) = "green" then report "image compares"; end if;
    if to_string(i) /= "6" then report "to_string compares"; end if;
    report "now=" & time'image(now);
    wait for 5 ns;
    report "now=" & time'image(now) & " min=" & integer'image(minimum(i, n));
    assert false;
    assert i = 4 severity warning;
    assert false report "with " & integer'image(i) severity note;
    wait;
  end process;
end architecture;
