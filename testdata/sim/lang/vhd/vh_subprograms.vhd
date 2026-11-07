library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  type pair_t is record
    lo, hi : integer;
  end record;
  signal sig_a : std_logic_vector(7 downto 0) := x"00";
  signal sig_cnt : integer := 0;

  function add3(a : integer; b : integer := 10; c : integer := 100) return integer is
  begin
    return a + b + c;
  end function;

  function fact(n : natural) return natural is
  begin
    if n <= 1 then
      return 1;
    else
      return n * fact(n - 1);
    end if;
  end function;

  function popcount(v : std_logic_vector) return natural is
    variable c : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        c := c + 1;
      end if;
    end loop;
    return c;
  end function;

  function reverse(v : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(v'reverse_range);
  begin
    for i in v'range loop
      r(i) := v(i);
    end loop;
    return r;
  end function;

  function minmax(a, b : integer) return pair_t is
  begin
    if a < b then
      return (lo => a, hi => b);
    else
      return (lo => b, hi => a);
    end if;
  end function;

  function f(x : integer) return integer is
  begin
    return x * 2;
  end function;
  function f(x : std_logic_vector) return integer is
  begin
    return x'length;
  end function;
  function f(x : boolean) return integer is
  begin
    if x then return 1; else return 0; end if;
  end function;

  procedure swap(variable a, b : inout integer) is
    variable t : integer;
  begin
    t := a; a := b; b := t;
  end procedure;

  procedure divmod(a, b : in integer; q, r : out integer) is
  begin
    q := a / b;
    r := a mod b;
  end procedure;

  procedure drive(signal s : out std_logic_vector; v : in std_logic_vector) is
  begin
    s <= v;
  end procedure;

  procedure pulse(signal c : inout integer; n : in natural) is
  begin
    for i in 1 to n loop
      c <= c + 1;
      wait for 1 ns;
    end loop;
  end procedure;

  impure function read_cnt return integer is
  begin
    return sig_cnt;
  end function;
begin
  process
    variable x, y, q, r : integer;
    variable p : pair_t;
  begin
    if add3(1) = 111 and add3(1, 2) = 103 and add3(1, c => 0) = 11 then report "defaults ok"; else report "defaults bad"; end if;
    if fact(6) = 720 then report "recursion ok"; else report "recursion bad"; end if;
    if popcount(x"F1") = 5 and popcount("101") = 2 then report "unconstrained param ok"; else report "unconstrained param bad"; end if;
    if reverse("1100") = "0011" then report "unconstrained return ok"; else report "unconstrained return bad"; end if;
    if reverse(x"01") = x"80" then report "unconstrained return 8 ok"; else report "unconstrained return 8 bad"; end if;
    p := minmax(9, 4);
    if p.lo = 4 and p.hi = 9 then report "record return ok"; else report "record return bad"; end if;
    if f(5) = 10 and f("1010") = 4 and f(true) = 1 then report "overload ok"; else report "overload bad"; end if;
    x := 1; y := 2;
    swap(x, y);
    if x = 2 and y = 1 then report "inout ok"; else report "inout bad"; end if;
    divmod(17, 5, q, r);
    if q = 3 and r = 2 then report "out params ok"; else report "out params bad"; end if;
    drive(sig_a, x"5A");
    wait for 1 ns;
    if sig_a = x"5A" then report "signal param ok"; else report "signal param bad"; end if;
    pulse(sig_cnt, 3);
    if sig_cnt = 3 then report "wait in procedure ok"; else report "wait in procedure bad"; end if;
    if read_cnt = 3 then report "impure ok"; else report "impure bad"; end if;
    report "fact(5)=" & integer'image(fact(5)); -- img
    wait;
  end process;
end architecture;
