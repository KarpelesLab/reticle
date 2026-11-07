library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  signal s : std_logic_vector(7 downto 0) := x"A5";
begin
  process
    variable i : integer := 7;
    variable u : unsigned(7 downto 0);
  begin
    report "start";
    if i + 3 = 10 then report "case1 ok"; else report "case1 bad"; end if;
    if s = x"A5" then report "case2 ok"; else report "case2 bad"; end if;
    u := unsigned(s) + 1;
    if u = x"A6" then report "case3 ok"; else report "case3 bad"; end if;
    if to_integer(u) = 166 then report "case4 ok"; else report "case4 bad"; end if;
    s <= not s;
    wait for 1 ns;
    if s = x"5A" then report "case5 ok"; else report "case5 bad"; end if;
    report "warn msg" severity warning;
    report "err msg" severity error;
    assert false report "assert note" severity note;
    assert i = 7 report "should not print" severity error;
    assert i = 8 report "assert warning" severity warning;
    assert false;
    report "end";
    wait;
  end process;
end architecture;
