library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  type color_t is (red, green, blue, yellow);
  type state_t is (IDLE, RUN, DONE);
  type mixed_t is ('a', 'b', foo, 'Z');
  subtype rg_t is color_t range red to green;
  signal st : state_t := IDLE;
begin
  process
    variable c : color_t := green;
  begin
    report "c=" & color_t'image(c);
    report "pos(blue)=" & integer'image(color_t'pos(blue));
    report "val(3)=" & color_t'image(color_t'val(3));
    report "succ(red)=" & color_t'image(color_t'succ(red));
    report "pred(yellow)=" & color_t'image(color_t'pred(yellow));
    report "left=" & color_t'image(color_t'left);
    report "right=" & color_t'image(color_t'right);
    report "high=" & color_t'image(color_t'high);
    report "low=" & color_t'image(color_t'low);
    report "value=" & color_t'image(color_t'value("BLUE"));
    report "rg high=" & color_t'image(rg_t'high);
    report "leftof(green)=" & color_t'image(color_t'leftof(green));
    report "rightof(green)=" & color_t'image(color_t'rightof(green));
    report "red<blue " & boolean'image(red < blue);
    report "max=" & color_t'image(maximum(red, yellow));
    report "mixed foo=" & mixed_t'image(foo);
    report "mixed 'a'=" & mixed_t'image('a');
    report "mixed pos Z=" & integer'image(mixed_t'pos('Z'));
    report "st=" & state_t'image(st);
    report "to_string(st)=" & to_string(st);
    report "bool=" & boolean'image(true) & " " & boolean'image(false);
    report "char=" & character'image('x');
    report "character'pos(A)=" & integer'image(character'pos('A'));
    report "character'val(66)=" & character'image(character'val(66));
    report "std_logic'pos('Z')=" & integer'image(std_logic'pos('Z'));
    report "bit'image=" & bit'image('1');
    report "severity=" & severity_level'image(warning);
    for k in color_t loop
      report "loop " & color_t'image(k);
    end loop;
    for k in color_t'high downto color_t'succ(color_t'low) loop
      report "rloop " & color_t'image(k);
    end loop;
    st <= state_t'succ(st);
    wait for 1 ns;
    report "st=" & state_t'image(st);
    wait;
  end process;
end architecture;
