library ieee;
use ieee.std_logic_1164.all;

package util_pkg is
  constant MAX_DEPTH : natural := 16#100#;
  subtype byte is std_logic_vector(7 downto 0);
  type state_t is (IDLE, RUN, DONE);

  function clog2(n : positive) return natural;
  function parity(v : std_logic_vector) return std_logic;
  procedure swap(a, b : inout byte);
end package util_pkg;

package body util_pkg is
  function clog2(n : positive) return natural is
    variable r : natural := 0;
    variable v : natural := n - 1;
  begin
    while v > 0 loop
      v := v / 2;
      r := r + 1;
    end loop;
    return r;
  end function clog2;

  function parity(v : std_logic_vector) return std_logic is
    variable p : std_logic := '0';
  begin
    for i in v'range loop
      p := p xor v(i);
    end loop;
    return p;
  end function;

  procedure swap(a, b : inout byte) is
    variable t : byte := a;
  begin
    a := b;
    b := t;
  end procedure;
end package body util_pkg;
