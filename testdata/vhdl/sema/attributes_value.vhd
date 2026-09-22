-- The value attributes: 'image, 'value, 'pos, 'val, 'succ, 'pred, 'base.
package val_pkg is
  type colour_t is (red, green, blue);

  constant P  : natural  := colour_t'pos(green);
  constant V  : colour_t := colour_t'val(2);
  constant S  : colour_t := colour_t'succ(red);
  constant R  : colour_t := colour_t'pred(blue);
  constant IM : string   := colour_t'image(blue);
  constant BK : colour_t := colour_t'value("red");

  constant NI : string  := integer'image(42);
  constant NV : integer := integer'value("17");
  constant NP : integer := integer'pos(5);

  subtype byte_t is natural range 0 to 255;
  constant BH : integer := byte_t'base'high;
end package val_pkg;

use work.val_pkg.all;

entity attr_value is
  port (q : out natural);
end entity;

architecture rtl of attr_value is
begin
  q <= P;
end architecture;
