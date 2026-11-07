library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  constant GREETING : string := "hello";
  constant S2 : string(1 to 3) := "abc";
  constant MSG : string := GREETING & " there";
begin
  process
  begin
    report GREETING;
    report MSG;
    report "static concat " & "of literals";
    report "image of constant " & integer'image(42);
    report "image of constant expr " & integer'image(6 * 7);
    report "image of character constant " & character'image('A');
    if GREETING(2) = 'e' then report "index ok"; else report "index bad"; end if;
    if GREETING(2 to 4) = "ell" then report "slice ok"; else report "slice bad"; end if;
    if GREETING = "hello" then report "eq ok"; else report "eq bad"; end if;
    if S2 < "abd" then report "lt ok"; else report "lt bad"; end if;
    if integer'image(42) = "42" then report "image eq ok"; else report "image eq bad"; end if;
    wait;
  end process;
end architecture;
