library ieee;
use ieee.std_logic_1164.all;

package util_pkg is
  function log2(n : positive) return natural;
  function "+"(a, b : std_logic_vector) return std_logic_vector;
  function max(a, b : integer := 0) return integer;
  impure function rand return real;
  pure function is_one(constant s : std_logic) return boolean;
  procedure swap(variable a, b : inout integer);
  procedure pulse(signal s : out std_logic; constant width : in time := 10 ns);
  procedure read_all(file f : text);

  -- VHDL-2008 generic subprograms.
  function generic_max
    generic (type t; function ">"(l, r : t) return boolean is <>)
    parameter (a, b : t) return t;
  function int_max is new generic_max generic map (t => integer);
  function int_max2 is new generic_max [integer, integer return integer]
    generic map (t => integer, ">" => ">");
end package;

package body util_pkg is
  function log2(n : positive) return natural is
    variable r : natural := 0;
    variable v : positive := n;
  begin
    while v > 1 loop
      v := v / 2;
      r := r + 1;
    end loop;
    return r;
  end function log2;

  function "+"(a, b : std_logic_vector) return std_logic_vector is
  begin
    return a xor b;
  end "+";

  function max(a, b : integer := 0) return integer is
  begin
    if a > b then
      return a;
    else
      return b;
    end if;
  end;

  impure function rand return real is
    variable seed1, seed2 : positive := 1;
    variable r : real;
  begin
    uniform(seed1, seed2, r);
    return r;
  end function;

  pure function is_one(constant s : std_logic) return boolean is
  begin
    return s = '1';
  end function is_one;

  procedure swap(variable a, b : inout integer) is
    variable t : integer := a;
  begin
    a := b;
    b := t;
  end procedure swap;

  procedure pulse(signal s : out std_logic; constant width : in time := 10 ns) is
  begin
    s <= '1', '0' after width;
    wait for width;
  end procedure;

  procedure read_all(file f : text) is
    variable l : line;
  begin
    while not endfile(f) loop
      readline(f, l);
    end loop;
  end;

  function generic_max
    generic (type t; function ">"(l, r : t) return boolean is <>)
    parameter (a, b : t) return t is
  begin
    if a > b then
      return a;
    end if;
    return b;
  end function;
end package body util_pkg;
