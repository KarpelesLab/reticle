-- The IEEE.STD_LOGIC_SIGNED package (a Synopsys legacy package that was
-- never an IEEE standard, but is compiled into the `ieee` library by every
-- vendor tool).
--
-- Clean-room source for Reticle. This file is an original implementation
-- of the package interface as the tools that ship it define it, written
-- for Reticle; it is not derived from any vendor source.
--
-- The package gives `std_logic_vector` the arithmetic of a signed
-- number. Reticle implements it with the same native core as
-- `numeric_std`, reading the signedness from the declaring package rather
-- than from the operand type. Every subprogram is `attribute foreign` and
-- the package has no body.

library ieee;
use ieee.std_logic_1164.all;

package std_logic_signed is

  function "+" (l, r : std_logic_vector) return std_logic_vector;
  function "+" (l : std_logic_vector; r : integer) return std_logic_vector;
  function "+" (l : integer; r : std_logic_vector) return std_logic_vector;
  function "+" (l : std_logic_vector; r : std_ulogic) return std_logic_vector;
  function "+" (l : std_ulogic; r : std_logic_vector) return std_logic_vector;

  function "-" (l, r : std_logic_vector) return std_logic_vector;
  function "-" (l : std_logic_vector; r : integer) return std_logic_vector;
  function "-" (l : integer; r : std_logic_vector) return std_logic_vector;
  function "-" (l : std_logic_vector; r : std_ulogic) return std_logic_vector;
  function "-" (l : std_ulogic; r : std_logic_vector) return std_logic_vector;

  function "+"   (l : std_logic_vector) return std_logic_vector;
  function "-"   (l : std_logic_vector) return std_logic_vector;
  function "abs" (l : std_logic_vector) return std_logic_vector;
  function "*"   (l, r : std_logic_vector) return std_logic_vector;

  function "<"  (l, r : std_logic_vector) return boolean;
  function "<"  (l : std_logic_vector; r : integer) return boolean;
  function "<"  (l : integer; r : std_logic_vector) return boolean;
  function "<=" (l, r : std_logic_vector) return boolean;
  function "<=" (l : std_logic_vector; r : integer) return boolean;
  function "<=" (l : integer; r : std_logic_vector) return boolean;
  function ">"  (l, r : std_logic_vector) return boolean;
  function ">"  (l : std_logic_vector; r : integer) return boolean;
  function ">"  (l : integer; r : std_logic_vector) return boolean;
  function ">=" (l, r : std_logic_vector) return boolean;
  function ">=" (l : std_logic_vector; r : integer) return boolean;
  function ">=" (l : integer; r : std_logic_vector) return boolean;
  function "="  (l : std_logic_vector; r : integer) return boolean;
  function "="  (l : integer; r : std_logic_vector) return boolean;
  function "/=" (l : std_logic_vector; r : integer) return boolean;
  function "/=" (l : integer; r : std_logic_vector) return boolean;

  function conv_integer (arg : std_logic_vector) return integer;

  attribute foreign of "+"  : function is "reticle: builtin";
  attribute foreign of "-"  : function is "reticle: builtin";
  attribute foreign of "*"  : function is "reticle: builtin";
  attribute foreign of "abs" : function is "reticle: builtin";
  attribute foreign of "<"  : function is "reticle: builtin";
  attribute foreign of "<=" : function is "reticle: builtin";
  attribute foreign of ">"  : function is "reticle: builtin";
  attribute foreign of ">=" : function is "reticle: builtin";
  attribute foreign of "="  : function is "reticle: builtin";
  attribute foreign of "/=" : function is "reticle: builtin";
  attribute foreign of conv_integer : function is "reticle: builtin";

end package std_logic_signed;
