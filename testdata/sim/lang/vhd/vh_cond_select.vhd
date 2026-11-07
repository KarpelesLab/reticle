library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal sel : std_logic_vector(1 downto 0) := "00";
  signal a, b, c, d : std_logic_vector(3 downto 0);
  signal y_when, y_with, y_sel_q : std_logic_vector(3 downto 0);
  signal en : std_logic := '0';
  signal y_en : std_logic_vector(3 downto 0);
  signal prio : std_logic_vector(3 downto 0) := "0000";
  signal enc : integer;
begin
  a <= x"A"; b <= x"B"; c <= x"C"; d <= x"D";

  y_when <= a when sel = "00" else
            b when sel = "01" else
            c when sel = "10" else
            d;

  with sel select
    y_with <= a when "00",
              b when "01",
              c when "10" | "11",
              x"0" when others;

  with prio select?
    y_sel_q <= x"3" when "1---",
               x"2" when "01--",
               x"1" when "001-",
               x"0" when others;

  y_en <= a when en = '1' else (others => 'Z');

  enc <= 3 when prio(3) = '1' else 2 when prio(2) = '1' else 1 when prio(1) = '1' else 0;

  process
    variable v : std_logic_vector(3 downto 0);
    variable k : integer;
  begin
    wait for 1 ns;
    if y_when = x"A" and y_with = x"A" then report "sel 00 ok"; else report "sel 00 bad"; end if;
    if y_en = "ZZZZ" then report "tristate ok"; else report "tristate bad"; end if;
    sel <= "10"; en <= '1'; prio <= "0110";
    wait for 1 ns;
    if y_when = x"C" and y_with = x"C" then report "sel 10 ok"; else report "sel 10 bad"; end if;
    if y_en = x"A" then report "enable ok"; else report "enable bad"; end if;
    if y_sel_q = x"2" then report "select? ok"; else report "select? bad"; end if;
    if enc = 2 then report "priority when ok"; else report "priority when bad"; end if;
    sel <= "11"; prio <= "0001";
    wait for 1 ns;
    if y_when = x"D" and y_with = x"C" then report "sel 11 ok"; else report "sel 11 bad"; end if;
    if y_sel_q = x"0" then report "select? others ok"; else report "select? others bad"; end if;
    -- sequential conditional and selected assignments (VHDL-2008)
    v := x"1" when sel = "11" else x"2";
    if v = x"1" then report "seq when var ok"; else report "seq when var bad"; end if;
    with sel select
      k := 5 when "11",
           6 when others;
    if k = 5 then report "seq select var ok"; else report "seq select var bad"; end if;
    y_en <= x"F" when sel = "00" else x"E";
    wait for 1 ns;
    if y_en = x"E" then report "seq when sig ok"; else report "seq when sig bad"; end if;
    wait;
  end process;
end architecture;
