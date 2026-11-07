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
  begin
    if to_integer(ua + ub) = 213 then report "add ok"; else report "add bad"; end if;
    if to_integer(ub - ua) = 69 then report "sub wrap ok"; else report "sub wrap bad"; end if;
    u16 := ua * ub;
    if to_integer(u16) = 2600 then report "mul ok"; else report "mul bad"; end if;
    if to_integer(ua / ub) = 15 then report "div ok"; else report "div bad"; end if;
    if to_integer(ua mod ub) = 5 then report "mod ok"; else report "mod bad"; end if;
    if to_integer(sa / sb) = -14 then report "sdiv ok"; else report "sdiv bad"; end if;
    if to_integer(sa mod sb) = 5 then report "smod ok"; else report "smod bad"; end if;
    if to_integer(sa rem sb) = -2 then report "srem ok"; else report "srem bad"; end if;
    s16 := sa * sb;
    if to_integer(s16) = -700 then report "smul ok"; else report "smul bad"; end if;
    if to_integer(abs sa) = 100 then report "abs ok"; else report "abs bad"; end if;
    if resize(sa, 4) = "1100" then report "resize s4 ok"; else report "resize s4 bad"; end if;
    if resize(sa, 12) = x"F9C" then report "resize s12 ok"; else report "resize s12 bad"; end if;
    if shift_right(sa, 2) = "11100111" then report "shr s ok"; else report "shr s bad"; end if;
    if shift_right(ua, 2) = "00110010" then report "shr u ok"; else report "shr u bad"; end if;
    if rotate_left(ua, 1) = "10010001" then report "rotl ok"; else report "rotl bad"; end if;
    if (ua sll 1) = "10010000" then report "sll ok"; else report "sll bad"; end if;
    if to_unsigned(5, 8) = to_unsigned(5, 4) then report "diffwidth eq ok"; else report "diffwidth eq bad"; end if;
    if to_signed(-1, 4) = to_signed(-1, 8) then report "sdiffwidth eq ok"; else report "sdiffwidth eq bad"; end if;
    if ua > 199 then report "u>int ok"; else report "u>int bad"; end if;
    if sa < -99 then report "s<int ok"; else report "s<int bad"; end if;
    if to_unsigned(3, 2) < to_unsigned(4, 8) then report "lt diffwidth ok"; else report "lt diffwidth bad"; end if;
    if u16'length = 16 then report "len16 ok"; else report "len16 bad"; end if;
    wait;
  end process;
end architecture;
