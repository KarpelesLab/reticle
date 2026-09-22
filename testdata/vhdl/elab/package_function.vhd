-- A package function is inlined at the call site; in a concurrent context
-- the body goes into a generated combinational process.
package fn_pkg is
  function parity (v : bit_vector(7 downto 0)) return bit;
end package;

package body fn_pkg is
  function parity (v : bit_vector(7 downto 0)) return bit is
    variable t : bit := '0';
  begin
    for i in v'range loop
      t := t xor v(i);
    end loop;
    return t;
  end function;
end package body;

use work.fn_pkg.all;

entity package_function is
  port (
    d : in  bit_vector(7 downto 0);
    p : out bit
  );
end entity;

architecture rtl of package_function is
begin
  p <= parity(d);
end architecture;
