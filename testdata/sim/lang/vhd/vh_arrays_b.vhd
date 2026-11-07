library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  type int_arr is array (natural range <>) of integer;
  type word_arr is array (0 to 3) of std_logic_vector(7 downto 0);
  type mat_t is array (0 to 2, 0 to 3) of integer;
  type down_arr is array (7 downto 4) of integer;
  type byte_mem is array (natural range <>) of std_logic_vector(7 downto 0);
  subtype nib_arr is int_arr(1 to 4);
  signal words : word_arr := (x"11", x"22", x"33", x"44");
  signal mem : byte_mem(0 to 7) := (others => x"00");
begin
  process
    variable a : int_arr(0 to 4) := (10, 20, 30, 40, 50);
    variable d : down_arr := (1, 2, 3, 4);
    variable m : mat_t;
    variable n : nib_arr := (others => 7);
    variable s : integer;
    variable bv : std_logic_vector(31 downto 0) := x"DEADBEEF";
    variable nul : int_arr(1 to 0);
  begin
    if a(2) = 30 then report "index ok"; else report "index bad"; end if;
    if a'length = 5 and a'left = 0 and a'right = 4 then report "attr asc ok"; else report "attr asc bad"; end if;
    if d'left = 7 and d'right = 4 and d'high = 7 and d'low = 4 then report "attr desc ok"; else report "attr desc bad"; end if;
    if d(7) = 1 and d(4) = 4 then report "desc agg ok"; else report "desc agg bad"; end if;
    if d'ascending then report "ascending bad"; else report "ascending ok"; end if;
    s := 0;
    for i in a'range loop
      s := s + a(i);
    end loop;
    if s = 150 then report "range loop ok"; else report "range loop bad"; end if;
    s := 0;
    for i in a'reverse_range loop
      s := s * 2 + i;
    end loop;
    if s = 57 then report "reverse_range ok"; else report "reverse_range bad"; end if;
    for i in 0 to 2 loop
      for j in 0 to 3 loop
        m(i, j) := i * 10 + j;
      end loop;
    end loop;
    if n(1) = 7 and n(4) = 7 and n'left = 1 then report "subtype others ok"; else report "subtype others bad"; end if;
    if words(2) = x"33" then report "word arr ok"; else report "word arr bad"; end if;
    if words(1)(3 downto 0) = x"2" then report "arr of vec slice ok"; else report "arr of vec slice bad"; end if;
    if bv(15 downto 8) = x"BE" then report "slice ok"; else report "slice bad"; end if;
    if nul'length = 0 then report "null array ok"; else report "null array bad"; end if;
    if bv(3 downto 4)'length = 0 then report "null slice ok"; else report "null slice bad"; end if;
    a(1 to 3) := a(2 to 4);
    if a(1) = 30 and a(3) = 50 then report "slice assign ok"; else report "slice assign bad"; end if;
    mem(3) <= x"A5";
    mem(4)(7) <= '1';
    words(0 to 1) <= words(2 to 3);
    wait for 1 ns;
    if mem(3) = x"A5" and mem(4) = x"80" then report "sig elem ok"; else report "sig elem bad"; end if;
    if words(0) = x"33" and words(1) = x"44" then report "sig slice ok"; else report "sig slice bad"; end if;
    s := 0;
    for i in 0 to 3 loop
      if bv(8 * i + 7 downto 8 * i) = x"EF" then
        s := i;
      end if;
    end loop;
    if s = 0 then report "loop slice ok"; else report "loop slice bad"; end if;
    wait;
  end process;
end architecture;
