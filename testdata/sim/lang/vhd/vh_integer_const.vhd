library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  subtype small_t is integer range -10 to 10;
begin
  process
    variable a : integer := -7;
    variable b : integer := 3;
    variable k : integer;
  begin
    if a mod b = 2 then report "m1 ok"; else report "m1 bad"; end if;
    if 7 mod (-3) = -2 then report "m2 ok"; else report "m2 bad"; end if;
    if a mod (-b) = -1 then report "m3 ok"; else report "m3 bad"; end if;
    if a rem b = -1 then report "r1 ok"; else report "r1 bad"; end if;
    if 7 rem (-3) = 1 then report "r2 ok"; else report "r2 bad"; end if;
    if a / b = -2 then report "d1 ok"; else report "d1 bad"; end if;
    if 2 ** 10 = 1024 then report "p1 ok"; else report "p1 bad"; end if;
    if abs a = 7 then report "abs ok"; else report "abs bad"; end if;
    if integer'high = 2147483647 then report "high ok"; else report "high bad"; end if;
    if integer'low = -2147483648 then report "low ok"; else report "low bad"; end if;
    if small_t'high = 10 then report "sub ok"; else report "sub bad"; end if;
    if 16#FF# = 255 then report "based ok"; else report "based bad"; end if;
    k := 0;
    for i in small_t range -3 to 3 loop
      k := k + i * i;
    end loop;
    if k = 28 then report "loop ok"; else report "loop bad"; end if;
    if integer'value("42") = 42 then report "value ok"; else report "value bad"; end if;
    wait;
  end process;
end architecture;
