-- The IEEE.NUMERIC_STD_UNSIGNED package (IEEE 1076-2008 clause 16.8.5.3).
--
-- Clean-room source for Reticle: the declarations are those the standard
-- prescribes for the package, written for Reticle. The package gives
-- `std_ulogic_vector` (and so `std_logic_vector`) the arithmetic of
-- `numeric_std`'s `unsigned`. Reticle implements it with the same native
-- core as `numeric_std`, the signedness coming from the package, as it
-- does for the Synopsys `std_logic_unsigned`. Every subprogram is
-- `attribute foreign` and the package has no body.

library ieee;
use ieee.std_logic_1164.all;

package numeric_std_unsigned is

  function "+" (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "+" (l : std_ulogic_vector; r : natural) return std_ulogic_vector;
  function "+" (l : natural; r : std_ulogic_vector) return std_ulogic_vector;
  function "-" (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "-" (l : std_ulogic_vector; r : natural) return std_ulogic_vector;
  function "-" (l : natural; r : std_ulogic_vector) return std_ulogic_vector;
  function "*" (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "*" (l : std_ulogic_vector; r : natural) return std_ulogic_vector;
  function "*" (l : natural; r : std_ulogic_vector) return std_ulogic_vector;
  function "/" (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "/" (l : std_ulogic_vector; r : natural) return std_ulogic_vector;
  function "/" (l : natural; r : std_ulogic_vector) return std_ulogic_vector;
  function "rem" (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "rem" (l : std_ulogic_vector; r : natural) return std_ulogic_vector;
  function "rem" (l : natural; r : std_ulogic_vector) return std_ulogic_vector;
  function "mod" (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "mod" (l : std_ulogic_vector; r : natural) return std_ulogic_vector;
  function "mod" (l : natural; r : std_ulogic_vector) return std_ulogic_vector;

  function "<"  (l, r : std_ulogic_vector) return boolean;
  function "<"  (l : std_ulogic_vector; r : natural) return boolean;
  function "<"  (l : natural; r : std_ulogic_vector) return boolean;
  function "<=" (l, r : std_ulogic_vector) return boolean;
  function "<=" (l : std_ulogic_vector; r : natural) return boolean;
  function "<=" (l : natural; r : std_ulogic_vector) return boolean;
  function ">"  (l, r : std_ulogic_vector) return boolean;
  function ">"  (l : std_ulogic_vector; r : natural) return boolean;
  function ">"  (l : natural; r : std_ulogic_vector) return boolean;
  function ">=" (l, r : std_ulogic_vector) return boolean;
  function ">=" (l : std_ulogic_vector; r : natural) return boolean;
  function ">=" (l : natural; r : std_ulogic_vector) return boolean;
  function "="  (l, r : std_ulogic_vector) return boolean;
  function "="  (l : std_ulogic_vector; r : natural) return boolean;
  function "="  (l : natural; r : std_ulogic_vector) return boolean;
  function "/=" (l, r : std_ulogic_vector) return boolean;
  function "/=" (l : std_ulogic_vector; r : natural) return boolean;
  function "/=" (l : natural; r : std_ulogic_vector) return boolean;

  function minimum (l, r : std_ulogic_vector) return std_ulogic_vector;
  function maximum (l, r : std_ulogic_vector) return std_ulogic_vector;

  function shift_left  (arg : std_ulogic_vector; count : natural) return std_ulogic_vector;
  function shift_right (arg : std_ulogic_vector; count : natural) return std_ulogic_vector;
  function rotate_left  (arg : std_ulogic_vector; count : natural) return std_ulogic_vector;
  function rotate_right (arg : std_ulogic_vector; count : natural) return std_ulogic_vector;

  function resize (arg : std_ulogic_vector; new_size : natural) return std_ulogic_vector;

  function to_integer (arg : std_ulogic_vector) return natural;
  function to_stdlogicvector (arg, size : natural) return std_logic_vector;
  function to_std_logic_vector (arg, size : natural) return std_logic_vector;
  function to_slv (arg, size : natural) return std_logic_vector;
  function to_stdulogicvector (arg, size : natural) return std_ulogic_vector;
  function to_std_ulogic_vector (arg, size : natural) return std_ulogic_vector;
  function to_sulv (arg, size : natural) return std_ulogic_vector;

  attribute foreign of "+"   : function is "reticle: builtin";
  attribute foreign of "-"   : function is "reticle: builtin";
  attribute foreign of "*"   : function is "reticle: builtin";
  attribute foreign of "/"   : function is "reticle: builtin";
  attribute foreign of "rem" : function is "reticle: builtin";
  attribute foreign of "mod" : function is "reticle: builtin";
  attribute foreign of "<"   : function is "reticle: builtin";
  attribute foreign of "<="  : function is "reticle: builtin";
  attribute foreign of ">"   : function is "reticle: builtin";
  attribute foreign of ">="  : function is "reticle: builtin";
  attribute foreign of "="   : function is "reticle: builtin";
  attribute foreign of "/="  : function is "reticle: builtin";
  attribute foreign of minimum : function is "reticle: builtin";
  attribute foreign of maximum : function is "reticle: builtin";
  attribute foreign of shift_left   : function is "reticle: builtin";
  attribute foreign of shift_right  : function is "reticle: builtin";
  attribute foreign of rotate_left  : function is "reticle: builtin";
  attribute foreign of rotate_right : function is "reticle: builtin";
  attribute foreign of resize       : function is "reticle: builtin";
  attribute foreign of to_integer   : function is "reticle: builtin";
  attribute foreign of to_stdlogicvector    : function is "reticle: builtin";
  attribute foreign of to_std_logic_vector  : function is "reticle: builtin";
  attribute foreign of to_slv               : function is "reticle: builtin";
  attribute foreign of to_stdulogicvector   : function is "reticle: builtin";
  attribute foreign of to_std_ulogic_vector : function is "reticle: builtin";
  attribute foreign of to_sulv              : function is "reticle: builtin";

end package numeric_std_unsigned;
