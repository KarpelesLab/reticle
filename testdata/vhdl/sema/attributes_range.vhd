-- The range and bound attributes on types and array objects.
package attr_pkg is
  type small_t is range 0 to 15;
  subtype tiny_t is small_t range 0 to 3;
  type vec_t is array (7 downto 0) of bit;
  type up_t  is array (1 to 4) of bit;

  constant SL : small_t := small_t'left;
  constant SR : small_t := small_t'right;
  constant SH : small_t := small_t'high;
  constant SO : small_t := small_t'low;
  constant SA : boolean := small_t'ascending;
  constant TL : tiny_t  := tiny_t'high;

  constant VL : natural := vec_t'length;
  constant VH : integer := vec_t'high;
  constant VO : integer := vec_t'low;
  constant VA : boolean := vec_t'ascending;
  constant UA : boolean := up_t'ascending;
end package attr_pkg;

use work.attr_pkg.all;

entity attr_range is
  port (d : in vec_t; q : out bit);
end entity;

architecture rtl of attr_range is
  signal acc : bit;
begin
  process (d)
    variable t : bit := '0';
  begin
    for i in d'range loop
      t := t xor d(i);
    end loop;
    for i in d'reverse_range loop
      t := t xor d(i);
    end loop;
    acc <= t;
  end process;

  q <= acc;
end architecture;
