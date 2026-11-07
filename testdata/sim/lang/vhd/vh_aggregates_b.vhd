library ieee;
use ieee.std_logic_1164.all;

entity tb is
end entity;

architecture sim of tb is
  type int_arr is array (0 to 7) of integer;
  signal a, b : std_logic;
  signal hi, lo : std_logic_vector(3 downto 0);
  signal v : std_logic_vector(7 downto 0) := x"A5";
  signal flags : std_logic_vector(3 downto 0);
begin
  (a, b) <= std_logic_vector'("10");
  (hi, lo) <= v;
  flags <= (3 => '1', 0 => '1', others => '0');

  process
    variable x : std_logic_vector(7 downto 0);
    variable ia : int_arr;
    variable p, q : std_logic;
    variable n1, n2 : std_logic_vector(1 downto 0);
  begin
    x := (others => '1');
    if x = x"FF" then report "others ok"; else report "others bad"; end if;
    x := (7 => '1', others => '0');
    if x = x"80" then report "named ok"; else report "named bad"; end if;
    x := (7 downto 4 => '1', others => '0');
    if x = x"F0" then report "range choice ok"; else report "range choice bad"; end if;
    x := ('1', '0', '1', '0', others => '0');
    if x = x"A0" then report "positional others ok"; else report "positional others bad"; end if;
    x := (1 | 3 | 5 => '1', others => '0');
    if x = x"2A" then report "choice list ok"; else report "choice list bad"; end if;
    ia := (0 to 3 => 5, 4 | 6 => 1, others => 9);
    if ia(2) = 5 and ia(4) = 1 and ia(5) = 9 and ia(6) = 1 and ia(7) = 9 then report "int agg ok"; else report "int agg bad"; end if;
    ia := (others => 0);
    ia(3) := 33;
    if ia(3) = 33 and ia(0) = 0 then report "int others ok"; else report "int others bad"; end if;
    (p, q) := std_logic_vector'("01");
    if p = '0' and q = '1' then report "var agg target ok"; else report "var agg target bad"; end if;
    (n1, n2) := std_logic_vector'("1100");
    if n1 = "11" and n2 = "00" then report "vec agg target ok"; else report "vec agg target bad"; end if;
    x := (x'high => '1', x'low => '1', others => '0');
    if x = x"81" then report "attr choice ok"; else report "attr choice bad"; end if;
    wait for 1 ns;
    if a = '1' and b = '0' then report "sig agg target ok"; else report "sig agg target bad"; end if;
    if hi = x"A" and lo = x"5" then report "sig vec agg target ok"; else report "sig vec agg target bad"; end if;
    if flags = "1001" then report "concurrent named ok"; else report "concurrent named bad"; end if;
    wait;
  end process;
end architecture;
