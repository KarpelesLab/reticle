library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package util_pkg is
  constant DATA_W : integer := 12;
  constant MAGIC : std_logic_vector(7 downto 0) := x"C9";
  constant DEFERRED : integer;
  subtype data_t is std_logic_vector(DATA_W - 1 downto 0);
  type op_t is (OP_ADD, OP_SUB, OP_XOR);
  type lut_t is array (0 to 3) of integer;
  constant LUT : lut_t := (2, 3, 5, 7);
  function clog2(n : positive) return natural;
  function apply(op : op_t; a, b : unsigned) return unsigned;
  procedure incr(variable v : inout integer; by : in integer := 1);
end package;

package body util_pkg is
  constant DEFERRED : integer := 42;
  function clog2(n : positive) return natural is
    variable r : natural := 0;
    variable v : natural := 1;
  begin
    while v < n loop
      v := v * 2;
      r := r + 1;
    end loop;
    return r;
  end function;
  function apply(op : op_t; a, b : unsigned) return unsigned is
  begin
    case op is
      when OP_ADD => return a + b;
      when OP_SUB => return a - b;
      when OP_XOR => return a xor b;
    end case;
  end function;
  procedure incr(variable v : inout integer; by : in integer := 1) is
  begin
    v := v + by;
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.util_pkg.all;

entity tb is
end entity;

architecture sim of tb is
  constant DEPTH_W : natural := clog2(100);
  signal d : data_t := (others => '1');
  signal addr : unsigned(clog2(16) - 1 downto 0);
begin
  process
    variable n : integer := 0;
    variable u : unsigned(7 downto 0);
  begin
    if DATA_W = 12 and d'length = 12 then report "pkg subtype ok"; else report "pkg subtype bad"; end if;
    if DEFERRED = 42 then report "deferred ok"; else report "deferred bad"; end if;
    if MAGIC = x"C9" then report "pkg const ok"; else report "pkg const bad"; end if;
    if DEPTH_W = 7 and clog2(1) = 0 and clog2(8) = 3 then report "pkg func ok"; else report "pkg func bad"; end if;
    if addr'length = 4 then report "func in range ok"; else report "func in range bad"; end if;
    if LUT(3) = 7 then report "pkg lut ok"; else report "pkg lut bad"; end if;
    u := apply(OP_SUB, to_unsigned(10, 8), to_unsigned(3, 8));
    if to_integer(u) = 7 then report "pkg enum func ok"; else report "pkg enum func bad"; end if;
    u := apply(OP_XOR, to_unsigned(15, 8), to_unsigned(5, 8));
    if to_integer(u) = 10 then report "pkg xor ok"; else report "pkg xor bad"; end if;
    incr(n);
    incr(n, 5);
    if n = 6 then report "pkg proc ok"; else report "pkg proc bad"; end if;
    if work.util_pkg.LUT(0) = 2 then report "selected name ok"; else report "selected name bad"; end if;
    if op_t'image(OP_XOR) = "op_xor" then report "pkg enum image ok"; else report "pkg enum image bad"; end if;
    wait;
  end process;
end architecture;
