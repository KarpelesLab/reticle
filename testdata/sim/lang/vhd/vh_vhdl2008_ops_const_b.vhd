library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal sel : std_logic := 'H';
  signal v : std_logic_vector(3 downto 0) := "1010";
begin
  process
    variable u : unsigned(3 downto 0) := "0101";
  begin
    if ('1' ?= 'H') = '1' then report "c1 ok"; else report "c1 bad"; end if;
    if ('0' ?= '-') = '1' then report "c2 ok"; else report "c2 bad"; end if;
    if ('1' ?= 'X') = 'X' then report "c3 ok"; else report "c3 bad"; end if;
    if (u ?< 6) = '1' then report "c6 ok"; else report "c6 bad"; end if;
    if (u ?> 5) = '0' then report "c7 ok"; else report "c7 bad"; end if;
    if (and v) = '0' then report "c8 ok"; else report "c8 bad"; end if;
    if (or v) = '1' then report "c9 ok"; else report "c9 bad"; end if;
    if (xor v) = '0' then report "c10 ok"; else report "c10 bad"; end if;
    if (nand v) = '1' then report "c11 ok"; else report "c11 bad"; end if;
    if (xnor v) = '1' then report "c12 ok"; else report "c12 bad"; end if;
    if ?? sel then report "c13 ok"; else report "c13 bad"; end if;
    if sel then report "c14 ok"; else report "c14 bad"; end if;
    if v(1) and v(3) then report "c15 ok"; else report "c15 bad"; end if;
    if not v(0) then report "c16 ok"; else report "c16 bad"; end if;
    if ?? 'X' then report "c18 bad"; else report "c18 ok"; end if;
    if maximum(3, 9) = 9 then report "c19 ok"; else report "c19 bad"; end if;
    wait;
  end process;
end architecture;
