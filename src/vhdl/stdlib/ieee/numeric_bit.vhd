-- The IEEE.NUMERIC_BIT package (IEEE 1076-2008 clause 16.10).
--
-- Clean-room source for Reticle. This file is an original implementation
-- of the package interface defined by the relevant IEEE standard, written
-- for Reticle; it is not derived from the IEEE source distribution.
--
-- The package is `numeric_std` over `bit` instead of `std_ulogic`, and
-- Reticle implements it with the same native core: `logic::Logic` holds
-- both, and the builtins pick the element encoding from the operand type.
-- Every subprogram is `attribute foreign`, so the package has no body.

package numeric_bit is

  type unsigned is array (natural range <>) of bit;
  type signed   is array (natural range <>) of bit;

  alias u_unsigned is unsigned;
  alias u_signed   is signed;

  ----------------------------------------------------------------------
  -- Sign and magnitude.
  ----------------------------------------------------------------------

  function "abs" (arg : signed) return signed;
  function "-"   (arg : signed) return signed;
  function "+"   (arg : signed) return signed;

  ----------------------------------------------------------------------
  -- Addition and subtraction.
  ----------------------------------------------------------------------

  function "+" (l, r : unsigned) return unsigned;
  function "+" (l, r : signed) return signed;
  function "+" (l : unsigned; r : natural) return unsigned;
  function "+" (l : natural; r : unsigned) return unsigned;
  function "+" (l : signed; r : integer) return signed;
  function "+" (l : integer; r : signed) return signed;

  function "-" (l, r : unsigned) return unsigned;
  function "-" (l, r : signed) return signed;
  function "-" (l : unsigned; r : natural) return unsigned;
  function "-" (l : natural; r : unsigned) return unsigned;
  function "-" (l : signed; r : integer) return signed;
  function "-" (l : integer; r : signed) return signed;

  ----------------------------------------------------------------------
  -- Multiplication, division and the two remainders.
  ----------------------------------------------------------------------

  function "*" (l, r : unsigned) return unsigned;
  function "*" (l, r : signed) return signed;
  function "*" (l : unsigned; r : natural) return unsigned;
  function "*" (l : natural; r : unsigned) return unsigned;
  function "*" (l : signed; r : integer) return signed;
  function "*" (l : integer; r : signed) return signed;

  function "/" (l, r : unsigned) return unsigned;
  function "/" (l, r : signed) return signed;
  function "/" (l : unsigned; r : natural) return unsigned;
  function "/" (l : natural; r : unsigned) return unsigned;
  function "/" (l : signed; r : integer) return signed;
  function "/" (l : integer; r : signed) return signed;

  function "rem" (l, r : unsigned) return unsigned;
  function "rem" (l, r : signed) return signed;
  function "rem" (l : unsigned; r : natural) return unsigned;
  function "rem" (l : natural; r : unsigned) return unsigned;
  function "rem" (l : signed; r : integer) return signed;
  function "rem" (l : integer; r : signed) return signed;

  function "mod" (l, r : unsigned) return unsigned;
  function "mod" (l, r : signed) return signed;
  function "mod" (l : unsigned; r : natural) return unsigned;
  function "mod" (l : natural; r : unsigned) return unsigned;
  function "mod" (l : signed; r : integer) return signed;
  function "mod" (l : integer; r : signed) return signed;

  ----------------------------------------------------------------------
  -- Ordering and equality. The operands need not be the same length.
  ----------------------------------------------------------------------

  function "=" (l, r : unsigned) return boolean;
  function "=" (l, r : signed) return boolean;
  function "=" (l : unsigned; r : natural) return boolean;
  function "=" (l : natural; r : unsigned) return boolean;
  function "=" (l : signed; r : integer) return boolean;
  function "=" (l : integer; r : signed) return boolean;

  function "/=" (l, r : unsigned) return boolean;
  function "/=" (l, r : signed) return boolean;
  function "/=" (l : unsigned; r : natural) return boolean;
  function "/=" (l : natural; r : unsigned) return boolean;
  function "/=" (l : signed; r : integer) return boolean;
  function "/=" (l : integer; r : signed) return boolean;

  function "<" (l, r : unsigned) return boolean;
  function "<" (l, r : signed) return boolean;
  function "<" (l : unsigned; r : natural) return boolean;
  function "<" (l : natural; r : unsigned) return boolean;
  function "<" (l : signed; r : integer) return boolean;
  function "<" (l : integer; r : signed) return boolean;

  function "<=" (l, r : unsigned) return boolean;
  function "<=" (l, r : signed) return boolean;
  function "<=" (l : unsigned; r : natural) return boolean;
  function "<=" (l : natural; r : unsigned) return boolean;
  function "<=" (l : signed; r : integer) return boolean;
  function "<=" (l : integer; r : signed) return boolean;

  function ">" (l, r : unsigned) return boolean;
  function ">" (l, r : signed) return boolean;
  function ">" (l : unsigned; r : natural) return boolean;
  function ">" (l : natural; r : unsigned) return boolean;
  function ">" (l : signed; r : integer) return boolean;
  function ">" (l : integer; r : signed) return boolean;

  function ">=" (l, r : unsigned) return boolean;
  function ">=" (l, r : signed) return boolean;
  function ">=" (l : unsigned; r : natural) return boolean;
  function ">=" (l : natural; r : unsigned) return boolean;
  function ">=" (l : signed; r : integer) return boolean;
  function ">=" (l : integer; r : signed) return boolean;

  ----------------------------------------------------------------------
  -- VHDL-2008: the matching relational operators, which answer with a
  -- `bit` so an unknown operand gives an unknown answer.
  ----------------------------------------------------------------------

  function "?=" (l, r : unsigned) return bit;
  function "?=" (l, r : signed) return bit;
  function "?=" (l : unsigned; r : natural) return bit;
  function "?=" (l : natural; r : unsigned) return bit;
  function "?=" (l : signed; r : integer) return bit;
  function "?=" (l : integer; r : signed) return bit;

  function "?/=" (l, r : unsigned) return bit;
  function "?/=" (l, r : signed) return bit;
  function "?/=" (l : unsigned; r : natural) return bit;
  function "?/=" (l : natural; r : unsigned) return bit;
  function "?/=" (l : signed; r : integer) return bit;
  function "?/=" (l : integer; r : signed) return bit;

  function "?<" (l, r : unsigned) return bit;
  function "?<" (l, r : signed) return bit;
  function "?<" (l : unsigned; r : natural) return bit;
  function "?<" (l : natural; r : unsigned) return bit;
  function "?<" (l : signed; r : integer) return bit;
  function "?<" (l : integer; r : signed) return bit;

  function "?<=" (l, r : unsigned) return bit;
  function "?<=" (l, r : signed) return bit;
  function "?<=" (l : unsigned; r : natural) return bit;
  function "?<=" (l : natural; r : unsigned) return bit;
  function "?<=" (l : signed; r : integer) return bit;
  function "?<=" (l : integer; r : signed) return bit;

  function "?>" (l, r : unsigned) return bit;
  function "?>" (l, r : signed) return bit;
  function "?>" (l : unsigned; r : natural) return bit;
  function "?>" (l : natural; r : unsigned) return bit;
  function "?>" (l : signed; r : integer) return bit;
  function "?>" (l : integer; r : signed) return bit;

  function "?>=" (l, r : unsigned) return bit;
  function "?>=" (l, r : signed) return bit;
  function "?>=" (l : unsigned; r : natural) return bit;
  function "?>=" (l : natural; r : unsigned) return bit;
  function "?>=" (l : signed; r : integer) return bit;
  function "?>=" (l : integer; r : signed) return bit;

  ----------------------------------------------------------------------
  -- Shifts and rotates, by name and as operators.
  ----------------------------------------------------------------------

  function shift_left   (arg : unsigned; count : natural) return unsigned;
  function shift_left   (arg : signed;   count : natural) return signed;
  function shift_right  (arg : unsigned; count : natural) return unsigned;
  function shift_right  (arg : signed;   count : natural) return signed;
  function rotate_left  (arg : unsigned; count : natural) return unsigned;
  function rotate_left  (arg : signed;   count : natural) return signed;
  function rotate_right (arg : unsigned; count : natural) return unsigned;
  function rotate_right (arg : signed;   count : natural) return signed;

  function "sll" (arg : unsigned; count : integer) return unsigned;
  function "sll" (arg : signed;   count : integer) return signed;
  function "srl" (arg : unsigned; count : integer) return unsigned;
  function "srl" (arg : signed;   count : integer) return signed;
  function "rol" (arg : unsigned; count : integer) return unsigned;
  function "rol" (arg : signed;   count : integer) return signed;
  function "ror" (arg : unsigned; count : integer) return unsigned;
  function "ror" (arg : signed;   count : integer) return signed;
  function "sla" (arg : unsigned; count : integer) return unsigned;
  function "sla" (arg : signed;   count : integer) return signed;
  function "sra" (arg : unsigned; count : integer) return unsigned;
  function "sra" (arg : signed;   count : integer) return signed;

  ----------------------------------------------------------------------
  -- VHDL-2008: the logical operators, element by element, as reductions,
  -- and mixed with a single `bit`.
  ----------------------------------------------------------------------

  function "not"  (l : unsigned) return unsigned;
  function "not"  (l : signed) return signed;
  function "and"  (l, r : unsigned) return unsigned;
  function "and"  (l, r : signed) return signed;
  function "or"   (l, r : unsigned) return unsigned;
  function "or"   (l, r : signed) return signed;
  function "nand" (l, r : unsigned) return unsigned;
  function "nand" (l, r : signed) return signed;
  function "nor"  (l, r : unsigned) return unsigned;
  function "nor"  (l, r : signed) return signed;
  function "xor"  (l, r : unsigned) return unsigned;
  function "xor"  (l, r : signed) return signed;
  function "xnor" (l, r : unsigned) return unsigned;
  function "xnor" (l, r : signed) return signed;

  function "and"  (l : bit; r : unsigned) return unsigned;
  function "and"  (l : unsigned; r : bit) return unsigned;
  function "and"  (l : bit; r : signed) return signed;
  function "and"  (l : signed; r : bit) return signed;
  function "or"   (l : bit; r : unsigned) return unsigned;
  function "or"   (l : unsigned; r : bit) return unsigned;
  function "or"   (l : bit; r : signed) return signed;
  function "or"   (l : signed; r : bit) return signed;
  function "nand" (l : bit; r : unsigned) return unsigned;
  function "nand" (l : unsigned; r : bit) return unsigned;
  function "nand" (l : bit; r : signed) return signed;
  function "nand" (l : signed; r : bit) return signed;
  function "nor"  (l : bit; r : unsigned) return unsigned;
  function "nor"  (l : unsigned; r : bit) return unsigned;
  function "nor"  (l : bit; r : signed) return signed;
  function "nor"  (l : signed; r : bit) return signed;
  function "xor"  (l : bit; r : unsigned) return unsigned;
  function "xor"  (l : unsigned; r : bit) return unsigned;
  function "xor"  (l : bit; r : signed) return signed;
  function "xor"  (l : signed; r : bit) return signed;
  function "xnor" (l : bit; r : unsigned) return unsigned;
  function "xnor" (l : unsigned; r : bit) return unsigned;
  function "xnor" (l : bit; r : signed) return signed;
  function "xnor" (l : signed; r : bit) return signed;

  function "and"  (l : unsigned) return bit;
  function "and"  (l : signed) return bit;
  function "or"   (l : unsigned) return bit;
  function "or"   (l : signed) return bit;
  function "nand" (l : unsigned) return bit;
  function "nand" (l : signed) return bit;
  function "nor"  (l : unsigned) return bit;
  function "nor"  (l : signed) return bit;
  function "xor"  (l : unsigned) return bit;
  function "xor"  (l : signed) return bit;
  function "xnor" (l : unsigned) return bit;
  function "xnor" (l : signed) return bit;

  ----------------------------------------------------------------------
  -- Resizing and conversion.
  ----------------------------------------------------------------------

  function resize (arg : unsigned; new_size : natural) return unsigned;
  function resize (arg : signed;   new_size : natural) return signed;
  function resize (arg : unsigned; size_res : unsigned) return unsigned;
  function resize (arg : signed;   size_res : signed) return signed;

  function to_integer  (arg : unsigned) return natural;
  function to_integer  (arg : signed) return integer;
  function to_unsigned (arg : natural; size : natural) return unsigned;
  function to_signed   (arg : integer; size : natural) return signed;
  function to_unsigned (arg : natural; size_res : unsigned) return unsigned;
  function to_signed   (arg : integer; size_res : signed) return signed;

  ----------------------------------------------------------------------
  -- VHDL-2008: extrema and bit search.
  ----------------------------------------------------------------------

  function minimum (l, r : unsigned) return unsigned;
  function minimum (l, r : signed) return signed;
  function minimum (l : unsigned; r : natural) return unsigned;
  function minimum (l : natural; r : unsigned) return unsigned;
  function minimum (l : signed; r : integer) return signed;
  function minimum (l : integer; r : signed) return signed;

  function maximum (l, r : unsigned) return unsigned;
  function maximum (l, r : signed) return signed;
  function maximum (l : unsigned; r : natural) return unsigned;
  function maximum (l : natural; r : unsigned) return unsigned;
  function maximum (l : signed; r : integer) return signed;
  function maximum (l : integer; r : signed) return signed;

  function find_leftmost  (arg : unsigned; y : bit) return integer;
  function find_leftmost  (arg : signed;   y : bit) return integer;
  function find_rightmost (arg : unsigned; y : bit) return integer;
  function find_rightmost (arg : signed;   y : bit) return integer;

  ----------------------------------------------------------------------
  -- VHDL-2008: string rendering.
  ----------------------------------------------------------------------

  function to_string  (value : unsigned) return string;
  function to_string  (value : signed) return string;
  function to_bstring (value : unsigned) return string;
  function to_bstring (value : signed) return string;
  function to_ostring (value : unsigned) return string;
  function to_ostring (value : signed) return string;
  function to_hstring (value : unsigned) return string;
  function to_hstring (value : signed) return string;

  ----------------------------------------------------------------------
  -- Every subprogram above is implemented natively. An attribute
  -- specification without a signature names every overload of the
  -- designator, which is why one line per name is enough.
  ----------------------------------------------------------------------

  attribute foreign of "abs"  : function is "reticle: builtin";
  attribute foreign of "+"    : function is "reticle: builtin";
  attribute foreign of "-"    : function is "reticle: builtin";
  attribute foreign of "*"    : function is "reticle: builtin";
  attribute foreign of "/"    : function is "reticle: builtin";
  attribute foreign of "rem"  : function is "reticle: builtin";
  attribute foreign of "mod"  : function is "reticle: builtin";
  attribute foreign of "="    : function is "reticle: builtin";
  attribute foreign of "/="   : function is "reticle: builtin";
  attribute foreign of "<"    : function is "reticle: builtin";
  attribute foreign of "<="   : function is "reticle: builtin";
  attribute foreign of ">"    : function is "reticle: builtin";
  attribute foreign of ">="   : function is "reticle: builtin";
  attribute foreign of "?="   : function is "reticle: builtin";
  attribute foreign of "?/="  : function is "reticle: builtin";
  attribute foreign of "?<"   : function is "reticle: builtin";
  attribute foreign of "?<="  : function is "reticle: builtin";
  attribute foreign of "?>"   : function is "reticle: builtin";
  attribute foreign of "?>="  : function is "reticle: builtin";
  attribute foreign of "not"  : function is "reticle: builtin";
  attribute foreign of "and"  : function is "reticle: builtin";
  attribute foreign of "or"   : function is "reticle: builtin";
  attribute foreign of "nand" : function is "reticle: builtin";
  attribute foreign of "nor"  : function is "reticle: builtin";
  attribute foreign of "xor"  : function is "reticle: builtin";
  attribute foreign of "xnor" : function is "reticle: builtin";
  attribute foreign of "sll"  : function is "reticle: builtin";
  attribute foreign of "srl"  : function is "reticle: builtin";
  attribute foreign of "rol"  : function is "reticle: builtin";
  attribute foreign of "ror"  : function is "reticle: builtin";
  attribute foreign of "sla"  : function is "reticle: builtin";
  attribute foreign of "sra"  : function is "reticle: builtin";

  attribute foreign of shift_left    : function is "reticle: builtin";
  attribute foreign of shift_right   : function is "reticle: builtin";
  attribute foreign of rotate_left   : function is "reticle: builtin";
  attribute foreign of rotate_right  : function is "reticle: builtin";
  attribute foreign of resize        : function is "reticle: builtin";
  attribute foreign of to_integer    : function is "reticle: builtin";
  attribute foreign of to_unsigned   : function is "reticle: builtin";
  attribute foreign of to_signed     : function is "reticle: builtin";
  attribute foreign of minimum       : function is "reticle: builtin";
  attribute foreign of maximum       : function is "reticle: builtin";
  attribute foreign of find_leftmost : function is "reticle: builtin";
  attribute foreign of find_rightmost: function is "reticle: builtin";
  attribute foreign of to_string     : function is "reticle: builtin";
  attribute foreign of to_bstring    : function is "reticle: builtin";
  attribute foreign of to_ostring    : function is "reticle: builtin";
  attribute foreign of to_hstring    : function is "reticle: builtin";

end package numeric_bit;
