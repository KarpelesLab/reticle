-- The IEEE.NUMERIC_STD package (IEEE 1076-2008 clause 16.9).
--
-- Clean-room source for Reticle. This file is an original implementation
-- of the package interface defined by the relevant IEEE standard, written
-- for Reticle; it is not derived from the IEEE source distribution.
--
-- Only the declarations are here. Every subprogram is marked `attribute
-- foreign` and implemented natively in Rust (`vhdl::sema::builtin` for the
-- static folding, `vhdl::elab::expr` for the lowering), over the same
-- `logic::Logic` value type the simulator uses. `unsigned` and `signed`
-- are arrays of `std_ulogic`, so a native body is both faster than
-- interpreting VHDL and exactly consistent with the rest of the compiler.
-- The package therefore has no body.

library ieee;
use ieee.std_logic_1164.all;

package numeric_std is

  type unresolved_unsigned is array (natural range <>) of std_ulogic;
  type unresolved_signed   is array (natural range <>) of std_ulogic;

  alias u_unsigned is unresolved_unsigned;
  alias u_signed   is unresolved_signed;

  subtype unsigned is (resolved) unresolved_unsigned;
  subtype signed   is (resolved) unresolved_signed;

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
  -- `std_ulogic` so an unknown operand gives an unknown answer.
  ----------------------------------------------------------------------

  function "?=" (l, r : unsigned) return std_ulogic;
  function "?=" (l, r : signed) return std_ulogic;
  function "?=" (l : unsigned; r : natural) return std_ulogic;
  function "?=" (l : natural; r : unsigned) return std_ulogic;
  function "?=" (l : signed; r : integer) return std_ulogic;
  function "?=" (l : integer; r : signed) return std_ulogic;

  function "?/=" (l, r : unsigned) return std_ulogic;
  function "?/=" (l, r : signed) return std_ulogic;
  function "?/=" (l : unsigned; r : natural) return std_ulogic;
  function "?/=" (l : natural; r : unsigned) return std_ulogic;
  function "?/=" (l : signed; r : integer) return std_ulogic;
  function "?/=" (l : integer; r : signed) return std_ulogic;

  function "?<" (l, r : unsigned) return std_ulogic;
  function "?<" (l, r : signed) return std_ulogic;
  function "?<" (l : unsigned; r : natural) return std_ulogic;
  function "?<" (l : natural; r : unsigned) return std_ulogic;
  function "?<" (l : signed; r : integer) return std_ulogic;
  function "?<" (l : integer; r : signed) return std_ulogic;

  function "?<=" (l, r : unsigned) return std_ulogic;
  function "?<=" (l, r : signed) return std_ulogic;
  function "?<=" (l : unsigned; r : natural) return std_ulogic;
  function "?<=" (l : natural; r : unsigned) return std_ulogic;
  function "?<=" (l : signed; r : integer) return std_ulogic;
  function "?<=" (l : integer; r : signed) return std_ulogic;

  function "?>" (l, r : unsigned) return std_ulogic;
  function "?>" (l, r : signed) return std_ulogic;
  function "?>" (l : unsigned; r : natural) return std_ulogic;
  function "?>" (l : natural; r : unsigned) return std_ulogic;
  function "?>" (l : signed; r : integer) return std_ulogic;
  function "?>" (l : integer; r : signed) return std_ulogic;

  function "?>=" (l, r : unsigned) return std_ulogic;
  function "?>=" (l, r : signed) return std_ulogic;
  function "?>=" (l : unsigned; r : natural) return std_ulogic;
  function "?>=" (l : natural; r : unsigned) return std_ulogic;
  function "?>=" (l : signed; r : integer) return std_ulogic;
  function "?>=" (l : integer; r : signed) return std_ulogic;

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
  -- and mixed with a single `std_ulogic`.
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

  function "and"  (l : std_ulogic; r : unsigned) return unsigned;
  function "and"  (l : unsigned; r : std_ulogic) return unsigned;
  function "and"  (l : std_ulogic; r : signed) return signed;
  function "and"  (l : signed; r : std_ulogic) return signed;
  function "or"   (l : std_ulogic; r : unsigned) return unsigned;
  function "or"   (l : unsigned; r : std_ulogic) return unsigned;
  function "or"   (l : std_ulogic; r : signed) return signed;
  function "or"   (l : signed; r : std_ulogic) return signed;
  function "nand" (l : std_ulogic; r : unsigned) return unsigned;
  function "nand" (l : unsigned; r : std_ulogic) return unsigned;
  function "nand" (l : std_ulogic; r : signed) return signed;
  function "nand" (l : signed; r : std_ulogic) return signed;
  function "nor"  (l : std_ulogic; r : unsigned) return unsigned;
  function "nor"  (l : unsigned; r : std_ulogic) return unsigned;
  function "nor"  (l : std_ulogic; r : signed) return signed;
  function "nor"  (l : signed; r : std_ulogic) return signed;
  function "xor"  (l : std_ulogic; r : unsigned) return unsigned;
  function "xor"  (l : unsigned; r : std_ulogic) return unsigned;
  function "xor"  (l : std_ulogic; r : signed) return signed;
  function "xor"  (l : signed; r : std_ulogic) return signed;
  function "xnor" (l : std_ulogic; r : unsigned) return unsigned;
  function "xnor" (l : unsigned; r : std_ulogic) return unsigned;
  function "xnor" (l : std_ulogic; r : signed) return signed;
  function "xnor" (l : signed; r : std_ulogic) return signed;

  function "and"  (l : unsigned) return std_ulogic;
  function "and"  (l : signed) return std_ulogic;
  function "or"   (l : unsigned) return std_ulogic;
  function "or"   (l : signed) return std_ulogic;
  function "nand" (l : unsigned) return std_ulogic;
  function "nand" (l : signed) return std_ulogic;
  function "nor"  (l : unsigned) return std_ulogic;
  function "nor"  (l : signed) return std_ulogic;
  function "xor"  (l : unsigned) return std_ulogic;
  function "xor"  (l : signed) return std_ulogic;
  function "xnor" (l : unsigned) return std_ulogic;
  function "xnor" (l : signed) return std_ulogic;

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
  -- Value inspection.
  ----------------------------------------------------------------------

  function to_01 (s : unsigned; xmap : std_ulogic := '0') return unsigned;
  function to_01 (s : signed;   xmap : std_ulogic := '0') return signed;

  function is_x (s : unsigned) return boolean;
  function is_x (s : signed) return boolean;

  function to_x01  (s : unsigned) return unsigned;
  function to_x01  (s : signed) return signed;
  function to_x01z (s : unsigned) return unsigned;
  function to_x01z (s : signed) return signed;
  function to_ux01 (s : unsigned) return unsigned;
  function to_ux01 (s : signed) return signed;

  -- `std_match` compares with `'-'` standing for any value, which is how
  -- a don't-care pattern is written.
  function std_match (l, r : std_ulogic) return boolean;
  function std_match (l, r : std_ulogic_vector) return boolean;
  function std_match (l, r : unsigned) return boolean;
  function std_match (l, r : signed) return boolean;

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

  function find_leftmost  (arg : unsigned; y : std_ulogic) return integer;
  function find_leftmost  (arg : signed;   y : std_ulogic) return integer;
  function find_rightmost (arg : unsigned; y : std_ulogic) return integer;
  function find_rightmost (arg : signed;   y : std_ulogic) return integer;

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
  attribute foreign of to_01         : function is "reticle: builtin";
  attribute foreign of to_x01        : function is "reticle: builtin";
  attribute foreign of to_x01z       : function is "reticle: builtin";
  attribute foreign of to_ux01       : function is "reticle: builtin";
  attribute foreign of is_x          : function is "reticle: builtin";
  attribute foreign of std_match     : function is "reticle: builtin";
  attribute foreign of minimum       : function is "reticle: builtin";
  attribute foreign of maximum       : function is "reticle: builtin";
  attribute foreign of find_leftmost : function is "reticle: builtin";
  attribute foreign of find_rightmost: function is "reticle: builtin";
  attribute foreign of to_string     : function is "reticle: builtin";
  attribute foreign of to_bstring    : function is "reticle: builtin";
  attribute foreign of to_ostring    : function is "reticle: builtin";
  attribute foreign of to_hstring    : function is "reticle: builtin";

end package numeric_std;
