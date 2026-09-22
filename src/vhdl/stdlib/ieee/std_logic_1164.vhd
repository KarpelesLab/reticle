-- The IEEE.STD_LOGIC_1164 package (IEEE 1164-1993, as revised by
-- IEEE 1076-2008 clause 16.8).
--
-- Clean-room source for Reticle: the declarations are the normative public
-- interface of the standard package; the bodies in `std_logic_1164_body.vhd`
-- are Reticle's own implementation.

package std_logic_1164 is

  -- The nine-state logic value system: uninitialised, forcing unknown,
  -- forcing low, forcing high, high impedance, weak unknown, weak low,
  -- weak high and don't care.
  type std_ulogic is ('U', 'X', '0', '1', 'Z', 'W', 'L', 'H', '-');

  type std_ulogic_vector is array (natural range <>) of std_ulogic;

  function resolved (s : std_ulogic_vector) return std_ulogic;

  subtype std_logic is resolved std_ulogic;

  -- Since the 2008 revision `std_logic_vector` is a resolved subtype of
  -- `std_ulogic_vector` rather than a type of its own, so the two are
  -- assignment-compatible without a conversion.
  subtype std_logic_vector is (resolved) std_ulogic_vector;

  subtype x01     is resolved std_ulogic range 'X' to '1';
  subtype x01z    is resolved std_ulogic range 'X' to 'Z';
  subtype ux01    is resolved std_ulogic range 'U' to '1';
  subtype ux01z   is resolved std_ulogic range 'U' to 'Z';

  ------------------------------------------------------------------------
  -- Logical operators on scalars.
  ------------------------------------------------------------------------

  function "and"  (l, r : std_ulogic) return ux01;
  function "nand" (l, r : std_ulogic) return ux01;
  function "or"   (l, r : std_ulogic) return ux01;
  function "nor"  (l, r : std_ulogic) return ux01;
  function "xor"  (l, r : std_ulogic) return ux01;
  function "xnor" (l, r : std_ulogic) return ux01;
  function "not"  (l : std_ulogic) return ux01;

  ------------------------------------------------------------------------
  -- Logical operators on vectors, element by element.
  ------------------------------------------------------------------------

  function "and"  (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "nand" (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "or"   (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "nor"  (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "xor"  (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "xnor" (l, r : std_ulogic_vector) return std_ulogic_vector;
  function "not"  (l : std_ulogic_vector) return std_ulogic_vector;

  ------------------------------------------------------------------------
  -- VHDL-2008: mixed scalar / vector forms.
  ------------------------------------------------------------------------

  function "and"  (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector;
  function "and"  (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector;
  function "nand" (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector;
  function "nand" (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector;
  function "or"   (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector;
  function "or"   (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector;
  function "nor"  (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector;
  function "nor"  (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector;
  function "xor"  (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector;
  function "xor"  (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector;
  function "xnor" (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector;
  function "xnor" (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector;

  ------------------------------------------------------------------------
  -- VHDL-2008: reduction operators.
  ------------------------------------------------------------------------

  function "and"  (l : std_ulogic_vector) return std_ulogic;
  function "nand" (l : std_ulogic_vector) return std_ulogic;
  function "or"   (l : std_ulogic_vector) return std_ulogic;
  function "nor"  (l : std_ulogic_vector) return std_ulogic;
  function "xor"  (l : std_ulogic_vector) return std_ulogic;
  function "xnor" (l : std_ulogic_vector) return std_ulogic;

  ------------------------------------------------------------------------
  -- VHDL-2008: shift and rotate.
  ------------------------------------------------------------------------

  function "sll" (l : std_ulogic_vector; r : integer) return std_ulogic_vector;
  function "srl" (l : std_ulogic_vector; r : integer) return std_ulogic_vector;
  function "rol" (l : std_ulogic_vector; r : integer) return std_ulogic_vector;
  function "ror" (l : std_ulogic_vector; r : integer) return std_ulogic_vector;

  ------------------------------------------------------------------------
  -- VHDL-2008: matching (condition) operators.
  ------------------------------------------------------------------------

  function "?="  (l, r : std_ulogic) return std_ulogic;
  function "?/=" (l, r : std_ulogic) return std_ulogic;
  function "?<"  (l, r : std_ulogic) return std_ulogic;
  function "?<=" (l, r : std_ulogic) return std_ulogic;
  function "?>"  (l, r : std_ulogic) return std_ulogic;
  function "?>=" (l, r : std_ulogic) return std_ulogic;
  function "?="  (l, r : std_ulogic_vector) return std_ulogic;
  function "?/=" (l, r : std_ulogic_vector) return std_ulogic;

  function "??" (l : std_ulogic) return boolean;

  ------------------------------------------------------------------------
  -- Conversions.
  ------------------------------------------------------------------------

  function to_bit       (s : std_ulogic; xmap : bit := '0') return bit;
  function to_bitvector (s : std_ulogic_vector; xmap : bit := '0') return bit_vector;
  function to_stdulogic       (b : bit) return std_ulogic;
  function to_stdlogicvector  (b : bit_vector) return std_logic_vector;
  function to_stdulogicvector (b : bit_vector) return std_ulogic_vector;

  -- VHDL-2008 spellings.
  function to_bit_vector        (s : std_ulogic_vector; xmap : bit := '0') return bit_vector;
  function to_std_logic_vector  (b : bit_vector) return std_logic_vector;
  function to_std_ulogic_vector (b : bit_vector) return std_ulogic_vector;

  ------------------------------------------------------------------------
  -- Strength strippers and type-state testers.
  ------------------------------------------------------------------------

  function to_x01  (s : std_ulogic_vector) return std_ulogic_vector;
  function to_x01  (s : std_ulogic) return x01;
  function to_x01  (b : bit_vector) return std_ulogic_vector;
  function to_x01  (b : bit) return x01;
  function to_x01z (s : std_ulogic_vector) return std_ulogic_vector;
  function to_x01z (s : std_ulogic) return x01z;
  function to_x01z (b : bit_vector) return std_ulogic_vector;
  function to_x01z (b : bit) return x01z;
  function to_ux01 (s : std_ulogic_vector) return std_ulogic_vector;
  function to_ux01 (s : std_ulogic) return ux01;
  function to_ux01 (b : bit_vector) return std_ulogic_vector;
  function to_ux01 (b : bit) return ux01;

  -- VHDL-2008: map every value to '0' or '1'.
  function to_01 (s : std_ulogic_vector; xmap : std_ulogic := '0') return std_ulogic_vector;
  function to_01 (s : std_ulogic; xmap : std_ulogic := '0') return std_ulogic;

  function is_x (s : std_ulogic_vector) return boolean;
  function is_x (s : std_ulogic) return boolean;

  ------------------------------------------------------------------------
  -- Edge detection.
  ------------------------------------------------------------------------

  function rising_edge  (signal s : std_ulogic) return boolean;
  function falling_edge (signal s : std_ulogic) return boolean;

  ------------------------------------------------------------------------
  -- VHDL-2008: string rendering.
  ------------------------------------------------------------------------

  function to_string  (value : std_ulogic) return string;
  function to_string  (value : std_ulogic_vector) return string;
  function to_bstring (value : std_ulogic_vector) return string;
  function to_ostring (value : std_ulogic_vector) return string;
  function to_hstring (value : std_ulogic_vector) return string;

end package std_logic_1164;
