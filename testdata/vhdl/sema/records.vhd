-- Record types, nested selection and record aggregates.
package rec_pkg is
  type point_t is record
    x : integer;
    y : integer;
  end record point_t;

  type box_t is record
    lo    : point_t;
    hi    : point_t;
    valid : boolean;
  end record;

  constant ORIGIN : point_t := (x => 0, y => 0);
  constant UNIT   : box_t   := (lo => ORIGIN, hi => (1, 1), valid => true);
end package rec_pkg;

use work.rec_pkg.all;

entity rec_user is
  port (
    b     : in  box_t;
    width : out integer
  );
end entity;

architecture rtl of rec_user is
  signal span : point_t;
begin
  span.x <= b.hi.x - b.lo.x;
  span.y <= b.hi.y - b.lo.y;
  width  <= span.x;
end architecture;
