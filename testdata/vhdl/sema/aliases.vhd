-- Object, type and subprogram aliases.
package alias_pkg is
  subtype word_t is bit_vector(15 downto 0);
  function parity (v : word_t) return bit;
  alias par is parity [word_t return bit];
end package alias_pkg;

package body alias_pkg is
  function parity (v : word_t) return bit is
    variable t : bit := '0';
  begin
    for i in v'range loop
      t := t xor v(i);
    end loop;
    return t;
  end function parity;
end package body alias_pkg;

use work.alias_pkg.all;

entity alias_user is
  port (
    d  : in  word_t;
    hi : out bit_vector(7 downto 0);
    p  : out bit
  );
end entity;

architecture rtl of alias_user is
  alias upper   : bit_vector(7 downto 0) is d(15 downto 8);
  alias top_bit : bit is d(15);
  alias small_t is bit;
begin
  hi <= upper;
  p  <= par(d) xor top_bit;
end architecture;
