-- Overloaded functions resolved by argument and result type.
package fn_pkg is
  function max (a, b : integer) return integer;
  function max (a, b : real) return real;
  function width (n : natural) return natural;
  function "+" (a, b : bit_vector) return bit_vector;
  impure function tick return integer;
end package fn_pkg;

package body fn_pkg is
  function max (a, b : integer) return integer is
  begin
    if a > b then
      return a;
    end if;
    return b;
  end function max;

  function max (a, b : real) return real is
  begin
    if a > b then
      return a;
    end if;
    return b;
  end function max;

  function width (n : natural) return natural is
    variable v : natural := n;
    variable r : natural := 0;
  begin
    while v > 0 loop
      v := v / 2;
      r := r + 1;
    end loop;
    return r;
  end function width;

  function "+" (a, b : bit_vector) return bit_vector is
  begin
    return a xor b;
  end function "+";

  impure function tick return integer is
  begin
    return 0;
  end function tick;
end package body fn_pkg;

use work.fn_pkg.all;

entity fn_user is
  port (q : out integer);
end entity;

architecture rtl of fn_user is
  constant A : integer := max(3, 7);
  constant B : real    := max(1.5, 2.5);
  constant W : natural := width(255);
  signal   v : bit_vector(3 downto 0);
begin
  q <= A + W;
  v <= "0011" + "0101";
end architecture;
