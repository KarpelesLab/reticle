-- Slices whose bounds move with a loop's parameter or come from a generic,
-- and jumps out of nested loops, held to what Vivado 2026.1's xsim
-- printed for this file (loop_slices_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity narrow is
  generic (W : positive := 8);
  port (a : in std_logic_vector(W downto 0);
        sv, ss : out std_logic_vector(W - 1 downto 0));
end entity;

architecture rtl of narrow is
begin
  ss <= a(W - 1 downto 0);
  process (a)
    variable v : std_logic_vector(W downto 0);
  begin
    v := a;
    sv <= v(W - 1 downto 0);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is end entity;

architecture sim of tb is
  type mem_t is array (0 to 3) of std_logic_vector(31 downto 0);
  type color_t is (red, green, blue, yellow);
  signal mem : mem_t := (others => (others => '0'));
  signal data : std_logic_vector(31 downto 0) := x"12345678";
  signal swapped : std_logic_vector(31 downto 0);
  signal we : std_logic_vector(3 downto 0) := "0101";
  signal a9 : std_logic_vector(8 downto 0) := "110000001";
  signal sv, ss : std_logic_vector(7 downto 0);
begin
  u : entity work.narrow port map (a9, sv, ss);

  swap : process (data)
  begin
    for i in 0 to 3 loop
      swapped(8 * i + 7 downto 8 * i) <= data(8 * (3 - i) + 7 downto 8 * (3 - i));
    end loop;
  end process;

  process
    variable vm : mem_t := (others => (others => '1'));
    variable v : std_logic_vector(31 downto 0);
    variable sum : unsigned(7 downto 0);
    variable n, count : integer;
  begin
    wait for 1 ns;
    report "swapped " & to_hstring(swapped) & " ss " & to_hstring(ss) & " sv " & to_hstring(sv);
    -- The byte-write loop, on a signal memory and a variable one.
    for i in 0 to 3 loop
      if we(i) = '1' then
        mem(2)(8 * i + 7 downto 8 * i) <= data(8 * i + 7 downto 8 * i);
        vm(1)(8 * i + 7 downto 8 * i) := data(8 * i + 7 downto 8 * i);
      end if;
    end loop;
    wait for 1 ns;
    report "mem(2) " & to_hstring(mem(2)) & " vm(1) " & to_hstring(vm(1));
    sum := (others => '0');
    for i in 0 to 3 loop
      sum := sum + unsigned(data(8 * i + 7 downto 8 * i));
    end loop;
    v := (others => '0');
    for i in 0 to 7 loop
      v(4 * i + 3 downto 4 * i) := std_logic_vector(to_unsigned(i, 4));
    end loop;
    report "sum " & to_hstring(sum) & " v " & to_hstring(v);
    for i in 3 downto 0 loop
      next when i = 2;
      exit when i = 0;
      v(i * 8 + 7 downto i * 8) := not v(i * 8 + 7 downto i * 8);
    end loop;
    report "inverted " & to_hstring(v);
    -- A range that depends on another loop's parameter, and on a variable.
    count := 0;
    for k in 0 to 3 loop
      for k2 in k to 3 loop
        count := count + 1;
      end loop;
    end loop;
    n := 5;
    for j in 1 to n loop
      n := n - 1;
      count := count + 1;
    end loop;
    report "count " & integer'image(count) & " n " & integer'image(n);
    -- Jumps out of an inner loop.
    count := 0;
    outer : for a in 0 to 5 loop
      inner : for b in 0 to 5 loop
        next outer when b > a;
        exit outer when a = 3;
        count := count + 1;
      end loop inner;
    end loop outer;
    report "outer jumps " & integer'image(count);
    count := 0;
    l1 : loop
      count := count + 1;
      l2 : while true loop
        exit l1 when count = 4;
        next l1;
      end loop l2;
    end loop l1;
    report "plain loops " & integer'image(count);
    for c in color_t loop
      report "colour " & color_t'image(c);
    end loop;
    for b in std_logic range '0' to '1' loop
      report "bit " & std_logic'image(b);
    end loop;
    wait;
  end process;
end architecture;
