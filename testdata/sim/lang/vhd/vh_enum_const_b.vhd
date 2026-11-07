library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  type color_t is (red, green, blue, yellow);
  subtype gb_t is color_t range green to blue;
begin
  process
    variable c : color_t := green;
    variable n : integer;
  begin
    if color_t'pos(blue) = 2 then report "pos ok"; else report "pos bad"; end if;
    if color_t'val(3) = yellow then report "val ok"; else report "val bad"; end if;
    if color_t'left = red then report "left ok"; else report "left bad"; end if;
    if color_t'high = yellow then report "high ok"; else report "high bad"; end if;
    if gb_t'low = green then report "sub low ok"; else report "sub low bad"; end if;
    if color_t'value("blue") = blue then report "value ok"; else report "value bad"; end if;
    if red < blue then report "cmp ok"; else report "cmp bad"; end if;
    n := 0;
    for k in color_t loop
    end loop;
    n := 0;
    for k in gb_t loop
      n := n + 1;
    end loop;
    wait;
  end process;
end architecture;
