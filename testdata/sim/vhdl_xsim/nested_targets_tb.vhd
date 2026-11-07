-- Nested targets: a field of a field, a field of an array element, an
-- element of a field, held to what Vivado 2026.1's xsim printed for this
-- file (nested_targets_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;

entity tb is end entity;

architecture sim of tb is
  type point_t is record
    x : integer;
    y : integer;
  end record;
  type shape_t is record
    p : point_t;
    flags : std_logic_vector(3 downto 0);
  end record;
  type pair_t is record
    a : integer;
    b : std_logic_vector(7 downto 0);
  end record;
  type pair_arr is array (0 to 2) of pair_t;
  signal r : pair_arr := (others => (a => 0, b => x"00"));
begin
  process
    variable o : shape_t := (p => (x => 1, y => 2), flags => "0000");
    variable v : pair_arr := (others => (a => 0, b => x"00"));
    variable i : integer := 2;
  begin
    o.p.x := 7;
    o.p.y := o.p.x + 1;
    o.flags(2) := '1';
    o.flags(i - 1) := '1';
    o.flags(0 downto 0) := "1";
    report "o " & integer'image(o.p.x) & " " & integer'image(o.p.y) & " " & to_string(o.flags);
    v(1).a := 5;
    v(2).b(7 downto 4) := x"C";
    v(2).b(0) := '1';
    report "v " & integer'image(v(1).a) & " " & to_hstring(v(2).b) & " " & integer'image(v(0).a);
    r(1).a <= 9;
    r(2).b <= x"A5";
    wait for 1 ns;
    report "r " & integer'image(r(1).a) & " " & to_hstring(r(2).b) & " " & to_hstring(r(1).b);
    wait;
  end process;
end architecture;
