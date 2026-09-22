-- The IEEE.STD_LOGIC_ARITH package (a Synopsys legacy package that was
-- never an IEEE standard, but is compiled into the `ieee` library by every
-- vendor tool and used by a great deal of older code).
--
-- Clean-room source for Reticle. This file is an original implementation
-- of the package interface as the tools that ship it define it, written
-- for Reticle; it is not derived from any vendor source.
--
-- The types are arrays of `std_logic` and behave like the `numeric_std`
-- pair of the same names, so Reticle implements both with one native
-- core. Every subprogram is `attribute foreign`, and the package has no
-- body.
--
-- Note that this package's `unsigned` and `signed` are distinct types
-- from `ieee.numeric_std`'s; a design that uses both packages at once has
-- to say which it means, exactly as it does with a vendor tool.

library ieee;
use ieee.std_logic_1164.all;

package std_logic_arith is

  type unsigned is array (natural range <>) of std_logic;
  type signed   is array (natural range <>) of std_logic;

  subtype small_int is integer range 0 to 1;

  function "+" (l, r : unsigned) return unsigned;
  function "+" (l, r : signed) return signed;
  function "+" (l : unsigned; r : integer) return unsigned;
  function "+" (l : integer; r : unsigned) return unsigned;
  function "+" (l : signed; r : integer) return signed;
  function "+" (l : integer; r : signed) return signed;

  function "-" (l, r : unsigned) return unsigned;
  function "-" (l, r : signed) return signed;
  function "-" (l : unsigned; r : integer) return unsigned;
  function "-" (l : integer; r : unsigned) return unsigned;
  function "-" (l : signed; r : integer) return signed;
  function "-" (l : integer; r : signed) return signed;

  function "+"   (l : signed) return signed;
  function "-"   (l : signed) return signed;
  function "abs" (l : signed) return signed;

  function "*" (l, r : unsigned) return unsigned;
  function "*" (l, r : signed) return signed;

  function "<"  (l, r : unsigned) return boolean;
  function "<"  (l, r : signed) return boolean;
  function "<"  (l : unsigned; r : integer) return boolean;
  function "<"  (l : integer; r : unsigned) return boolean;
  function "<"  (l : signed; r : integer) return boolean;
  function "<"  (l : integer; r : signed) return boolean;

  function "<=" (l, r : unsigned) return boolean;
  function "<=" (l, r : signed) return boolean;
  function "<=" (l : unsigned; r : integer) return boolean;
  function "<=" (l : integer; r : unsigned) return boolean;
  function "<=" (l : signed; r : integer) return boolean;
  function "<=" (l : integer; r : signed) return boolean;

  function ">"  (l, r : unsigned) return boolean;
  function ">"  (l, r : signed) return boolean;
  function ">"  (l : unsigned; r : integer) return boolean;
  function ">"  (l : integer; r : unsigned) return boolean;
  function ">"  (l : signed; r : integer) return boolean;
  function ">"  (l : integer; r : signed) return boolean;

  function ">=" (l, r : unsigned) return boolean;
  function ">=" (l, r : signed) return boolean;
  function ">=" (l : unsigned; r : integer) return boolean;
  function ">=" (l : integer; r : unsigned) return boolean;
  function ">=" (l : signed; r : integer) return boolean;
  function ">=" (l : integer; r : signed) return boolean;

  function "="  (l, r : unsigned) return boolean;
  function "="  (l, r : signed) return boolean;
  function "="  (l : unsigned; r : integer) return boolean;
  function "="  (l : integer; r : unsigned) return boolean;
  function "="  (l : signed; r : integer) return boolean;
  function "="  (l : integer; r : signed) return boolean;

  function "/=" (l, r : unsigned) return boolean;
  function "/=" (l, r : signed) return boolean;
  function "/=" (l : unsigned; r : integer) return boolean;
  function "/=" (l : integer; r : unsigned) return boolean;
  function "/=" (l : signed; r : integer) return boolean;
  function "/=" (l : integer; r : signed) return boolean;

  function shl (arg : unsigned; count : unsigned) return unsigned;
  function shl (arg : signed;   count : unsigned) return signed;
  function shr (arg : unsigned; count : unsigned) return unsigned;
  function shr (arg : signed;   count : unsigned) return signed;

  function conv_integer (arg : integer)    return integer;
  function conv_integer (arg : unsigned)   return integer;
  function conv_integer (arg : signed)     return integer;
  function conv_integer (arg : std_ulogic) return small_int;

  function conv_unsigned (arg : integer;    size : integer) return unsigned;
  function conv_unsigned (arg : unsigned;   size : integer) return unsigned;
  function conv_unsigned (arg : signed;     size : integer) return unsigned;
  function conv_unsigned (arg : std_ulogic; size : integer) return unsigned;

  function conv_signed (arg : integer;    size : integer) return signed;
  function conv_signed (arg : unsigned;   size : integer) return signed;
  function conv_signed (arg : signed;     size : integer) return signed;
  function conv_signed (arg : std_ulogic; size : integer) return signed;

  function conv_std_logic_vector (arg : integer;    size : integer) return std_logic_vector;
  function conv_std_logic_vector (arg : unsigned;   size : integer) return std_logic_vector;
  function conv_std_logic_vector (arg : signed;     size : integer) return std_logic_vector;
  function conv_std_logic_vector (arg : std_ulogic; size : integer) return std_logic_vector;

  -- Zero extension and sign extension to `size` bits.
  function ext  (arg : std_logic_vector; size : integer) return std_logic_vector;
  function sxt  (arg : std_logic_vector; size : integer) return std_logic_vector;

  attribute foreign of "+"   : function is "reticle: builtin";
  attribute foreign of "-"   : function is "reticle: builtin";
  attribute foreign of "*"   : function is "reticle: builtin";
  attribute foreign of "abs" : function is "reticle: builtin";
  attribute foreign of "<"   : function is "reticle: builtin";
  attribute foreign of "<="  : function is "reticle: builtin";
  attribute foreign of ">"   : function is "reticle: builtin";
  attribute foreign of ">="  : function is "reticle: builtin";
  attribute foreign of "="   : function is "reticle: builtin";
  attribute foreign of "/="  : function is "reticle: builtin";

  attribute foreign of shl : function is "reticle: builtin";
  attribute foreign of shr : function is "reticle: builtin";
  attribute foreign of ext : function is "reticle: builtin";
  attribute foreign of sxt : function is "reticle: builtin";

  attribute foreign of conv_integer           : function is "reticle: builtin";
  attribute foreign of conv_unsigned          : function is "reticle: builtin";
  attribute foreign of conv_signed            : function is "reticle: builtin";
  attribute foreign of conv_std_logic_vector  : function is "reticle: builtin";

end package std_logic_arith;
