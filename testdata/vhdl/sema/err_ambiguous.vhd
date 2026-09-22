-- Ambiguity: a literal shared by two enumeration types, and a call whose
-- result type the context does not pin down.
package amb_pkg is
  type t1 is (red, green, blue);
  type t2 is (red, blue, violet);

  function f (a : integer) return integer;
  function f (a : integer) return boolean;
end package amb_pkg;

package body amb_pkg is
  function f (a : integer) return integer is
  begin
    return a;
  end function;

  function f (a : integer) return boolean is
  begin
    return a > 0;
  end function;
end package body amb_pkg;

use work.amb_pkg.all;

entity err_ambiguous is
  port (ok : out boolean);
end entity;

architecture rtl of err_ambiguous is
  signal a, b : boolean;
begin
  -- `red` and `blue` are literals of both `t1` and `t2`.
  a <= red = blue;

  -- Both operands could be `integer` or `boolean`.
  b <= f(1) = f(2);

  ok <= a and b;
end architecture;
