-- Body of IEEE.STD_LOGIC_1164. Reticle's own implementation.
--
-- The tables are written out as constants so the semantics are visible and
-- auditable: `and_table` and friends are indexed by the two operands'
-- positions, exactly as the standard's tables are laid out.

package body std_logic_1164 is

  type stdlogic_table is array (std_ulogic, std_ulogic) of std_ulogic;
  type stdlogic_1d    is array (std_ulogic) of std_ulogic;

  --                    U    X    0    1    Z    W    L    H    -
  constant resolution_table : stdlogic_table := (
    ('U', 'U', 'U', 'U', 'U', 'U', 'U', 'U', 'U'),  -- U
    ('U', 'X', 'X', 'X', 'X', 'X', 'X', 'X', 'X'),  -- X
    ('U', 'X', '0', 'X', '0', '0', '0', '0', 'X'),  -- 0
    ('U', 'X', 'X', '1', '1', '1', '1', '1', 'X'),  -- 1
    ('U', 'X', '0', '1', 'Z', 'W', 'L', 'H', 'X'),  -- Z
    ('U', 'X', '0', '1', 'W', 'W', 'W', 'W', 'X'),  -- W
    ('U', 'X', '0', '1', 'L', 'W', 'L', 'W', 'X'),  -- L
    ('U', 'X', '0', '1', 'H', 'W', 'W', 'H', 'X'),  -- H
    ('U', 'X', 'X', 'X', 'X', 'X', 'X', 'X', 'X')); -- -

  constant and_table : stdlogic_table := (
    ('U', 'U', '0', 'U', 'U', 'U', '0', 'U', 'U'),  -- U
    ('U', 'X', '0', 'X', 'X', 'X', '0', 'X', 'X'),  -- X
    ('0', '0', '0', '0', '0', '0', '0', '0', '0'),  -- 0
    ('U', 'X', '0', '1', 'X', 'X', '0', '1', 'X'),  -- 1
    ('U', 'X', '0', 'X', 'X', 'X', '0', 'X', 'X'),  -- Z
    ('U', 'X', '0', 'X', 'X', 'X', '0', 'X', 'X'),  -- W
    ('0', '0', '0', '0', '0', '0', '0', '0', '0'),  -- L
    ('U', 'X', '0', '1', 'X', 'X', '0', '1', 'X'),  -- H
    ('U', 'X', '0', 'X', 'X', 'X', '0', 'X', 'X')); -- -

  constant or_table : stdlogic_table := (
    ('U', 'U', 'U', '1', 'U', 'U', 'U', '1', 'U'),  -- U
    ('U', 'X', 'X', '1', 'X', 'X', 'X', '1', 'X'),  -- X
    ('U', 'X', '0', '1', 'X', 'X', '0', '1', 'X'),  -- 0
    ('1', '1', '1', '1', '1', '1', '1', '1', '1'),  -- 1
    ('U', 'X', 'X', '1', 'X', 'X', 'X', '1', 'X'),  -- Z
    ('U', 'X', 'X', '1', 'X', 'X', 'X', '1', 'X'),  -- W
    ('U', 'X', '0', '1', 'X', 'X', '0', '1', 'X'),  -- L
    ('1', '1', '1', '1', '1', '1', '1', '1', '1'),  -- H
    ('U', 'X', 'X', '1', 'X', 'X', 'X', '1', 'X')); -- -

  constant xor_table : stdlogic_table := (
    ('U', 'U', 'U', 'U', 'U', 'U', 'U', 'U', 'U'),  -- U
    ('U', 'X', 'X', 'X', 'X', 'X', 'X', 'X', 'X'),  -- X
    ('U', 'X', '0', '1', 'X', 'X', '0', '1', 'X'),  -- 0
    ('U', 'X', '1', '0', 'X', 'X', '1', '0', 'X'),  -- 1
    ('U', 'X', 'X', 'X', 'X', 'X', 'X', 'X', 'X'),  -- Z
    ('U', 'X', 'X', 'X', 'X', 'X', 'X', 'X', 'X'),  -- W
    ('U', 'X', '0', '1', 'X', 'X', '0', '1', 'X'),  -- L
    ('U', 'X', '1', '0', 'X', 'X', '1', '0', 'X'),  -- H
    ('U', 'X', 'X', 'X', 'X', 'X', 'X', 'X', 'X')); -- -

  constant not_table : stdlogic_1d :=
    ('U', 'X', '1', '0', 'X', 'X', '1', '0', 'X');

  constant to_x01_table : stdlogic_1d :=
    ('X', 'X', '0', '1', 'X', 'X', '0', '1', 'X');

  constant to_x01z_table : stdlogic_1d :=
    ('X', 'X', '0', '1', 'Z', 'X', '0', '1', 'X');

  constant to_ux01_table : stdlogic_1d :=
    ('U', 'X', '0', '1', 'X', 'X', '0', '1', 'X');

  constant hex_digits : string (1 to 16) := "0123456789ABCDEF";

  ------------------------------------------------------------------------
  -- Resolution.
  ------------------------------------------------------------------------

  function resolved (s : std_ulogic_vector) return std_ulogic is
    variable result : std_ulogic := 'Z';
  begin
    if s'length = 1 then
      return s(s'low);
    end if;
    for i in s'range loop
      result := resolution_table(result, s(i));
    end loop;
    return result;
  end function resolved;

  ------------------------------------------------------------------------
  -- Scalar operators.
  ------------------------------------------------------------------------

  function "and" (l, r : std_ulogic) return ux01 is
  begin
    return and_table(l, r);
  end function "and";

  function "nand" (l, r : std_ulogic) return ux01 is
  begin
    return not_table(and_table(l, r));
  end function "nand";

  function "or" (l, r : std_ulogic) return ux01 is
  begin
    return or_table(l, r);
  end function "or";

  function "nor" (l, r : std_ulogic) return ux01 is
  begin
    return not_table(or_table(l, r));
  end function "nor";

  function "xor" (l, r : std_ulogic) return ux01 is
  begin
    return xor_table(l, r);
  end function "xor";

  function "xnor" (l, r : std_ulogic) return ux01 is
  begin
    return not_table(xor_table(l, r));
  end function "xnor";

  function "not" (l : std_ulogic) return ux01 is
  begin
    return not_table(l);
  end function "not";

  ------------------------------------------------------------------------
  -- Vector operators. Both operands must be the same length; the result
  -- takes the index range `0 to length-1`, as the standard prescribes.
  ------------------------------------------------------------------------

  function "and" (l, r : std_ulogic_vector) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    alias rv : std_ulogic_vector (1 to r'length) is r;
    variable result : std_ulogic_vector (1 to l'length);
  begin
    assert l'length = r'length
      report "std_logic_1164.""and"": operands of different lengths"
      severity failure;
    for i in result'range loop
      result(i) := and_table(lv(i), rv(i));
    end loop;
    return result;
  end function "and";

  function "nand" (l, r : std_ulogic_vector) return std_ulogic_vector is
  begin
    return not (l and r);
  end function "nand";

  function "or" (l, r : std_ulogic_vector) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    alias rv : std_ulogic_vector (1 to r'length) is r;
    variable result : std_ulogic_vector (1 to l'length);
  begin
    assert l'length = r'length
      report "std_logic_1164.""or"": operands of different lengths"
      severity failure;
    for i in result'range loop
      result(i) := or_table(lv(i), rv(i));
    end loop;
    return result;
  end function "or";

  function "nor" (l, r : std_ulogic_vector) return std_ulogic_vector is
  begin
    return not (l or r);
  end function "nor";

  function "xor" (l, r : std_ulogic_vector) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    alias rv : std_ulogic_vector (1 to r'length) is r;
    variable result : std_ulogic_vector (1 to l'length);
  begin
    assert l'length = r'length
      report "std_logic_1164.""xor"": operands of different lengths"
      severity failure;
    for i in result'range loop
      result(i) := xor_table(lv(i), rv(i));
    end loop;
    return result;
  end function "xor";

  function "xnor" (l, r : std_ulogic_vector) return std_ulogic_vector is
  begin
    return not (l xor r);
  end function "xnor";

  function "not" (l : std_ulogic_vector) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    variable result : std_ulogic_vector (1 to l'length);
  begin
    for i in result'range loop
      result(i) := not_table(lv(i));
    end loop;
    return result;
  end function "not";

  ------------------------------------------------------------------------
  -- Mixed scalar / vector forms (VHDL-2008).
  ------------------------------------------------------------------------

  function "and" (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector is
    alias rv : std_ulogic_vector (1 to r'length) is r;
    variable result : std_ulogic_vector (1 to r'length);
  begin
    for i in result'range loop
      result(i) := and_table(l, rv(i));
    end loop;
    return result;
  end function "and";

  function "and" (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    variable result : std_ulogic_vector (1 to l'length);
  begin
    for i in result'range loop
      result(i) := and_table(lv(i), r);
    end loop;
    return result;
  end function "and";

  function "nand" (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector is
  begin
    return not (l and r);
  end function "nand";

  function "nand" (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector is
  begin
    return not (l and r);
  end function "nand";

  function "or" (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector is
    alias rv : std_ulogic_vector (1 to r'length) is r;
    variable result : std_ulogic_vector (1 to r'length);
  begin
    for i in result'range loop
      result(i) := or_table(l, rv(i));
    end loop;
    return result;
  end function "or";

  function "or" (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    variable result : std_ulogic_vector (1 to l'length);
  begin
    for i in result'range loop
      result(i) := or_table(lv(i), r);
    end loop;
    return result;
  end function "or";

  function "nor" (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector is
  begin
    return not (l or r);
  end function "nor";

  function "nor" (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector is
  begin
    return not (l or r);
  end function "nor";

  function "xor" (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector is
    alias rv : std_ulogic_vector (1 to r'length) is r;
    variable result : std_ulogic_vector (1 to r'length);
  begin
    for i in result'range loop
      result(i) := xor_table(l, rv(i));
    end loop;
    return result;
  end function "xor";

  function "xor" (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    variable result : std_ulogic_vector (1 to l'length);
  begin
    for i in result'range loop
      result(i) := xor_table(lv(i), r);
    end loop;
    return result;
  end function "xor";

  function "xnor" (l : std_ulogic; r : std_ulogic_vector) return std_ulogic_vector is
  begin
    return not (l xor r);
  end function "xnor";

  function "xnor" (l : std_ulogic_vector; r : std_ulogic) return std_ulogic_vector is
  begin
    return not (l xor r);
  end function "xnor";

  ------------------------------------------------------------------------
  -- Reductions (VHDL-2008). An empty vector yields the identity element.
  ------------------------------------------------------------------------

  function "and" (l : std_ulogic_vector) return std_ulogic is
    variable result : std_ulogic := '1';
  begin
    for i in l'range loop
      result := and_table(result, l(i));
    end loop;
    return result;
  end function "and";

  function "nand" (l : std_ulogic_vector) return std_ulogic is
  begin
    return not_table(and l);
  end function "nand";

  function "or" (l : std_ulogic_vector) return std_ulogic is
    variable result : std_ulogic := '0';
  begin
    for i in l'range loop
      result := or_table(result, l(i));
    end loop;
    return result;
  end function "or";

  function "nor" (l : std_ulogic_vector) return std_ulogic is
  begin
    return not_table(or l);
  end function "nor";

  function "xor" (l : std_ulogic_vector) return std_ulogic is
    variable result : std_ulogic := '0';
  begin
    for i in l'range loop
      result := xor_table(result, l(i));
    end loop;
    return result;
  end function "xor";

  function "xnor" (l : std_ulogic_vector) return std_ulogic is
  begin
    return not_table(xor l);
  end function "xnor";

  ------------------------------------------------------------------------
  -- Shift and rotate (VHDL-2008). A negative count shifts the other way.
  ------------------------------------------------------------------------

  function "sll" (l : std_ulogic_vector; r : integer) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    variable result : std_ulogic_vector (1 to l'length) := (others => '0');
  begin
    if r < 0 then
      return lv srl (-r);
    end if;
    for i in result'range loop
      if i + r <= result'high then
        result(i) := lv(i + r);
      end if;
    end loop;
    return result;
  end function "sll";

  function "srl" (l : std_ulogic_vector; r : integer) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    variable result : std_ulogic_vector (1 to l'length) := (others => '0');
  begin
    if r < 0 then
      return lv sll (-r);
    end if;
    for i in result'range loop
      if i - r >= result'low then
        result(i) := lv(i - r);
      end if;
    end loop;
    return result;
  end function "srl";

  function "rol" (l : std_ulogic_vector; r : integer) return std_ulogic_vector is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    variable result : std_ulogic_vector (1 to l'length);
    variable amount : integer;
  begin
    if l'length = 0 then
      return lv;
    end if;
    amount := r mod l'length;
    for i in result'range loop
      result(i) := lv(((i - 1 + amount) mod l'length) + 1);
    end loop;
    return result;
  end function "rol";

  function "ror" (l : std_ulogic_vector; r : integer) return std_ulogic_vector is
  begin
    if l'length = 0 then
      return l;
    end if;
    return l rol (l'length - (r mod l'length));
  end function "ror";

  ------------------------------------------------------------------------
  -- Matching operators (VHDL-2008). A '-' on either side matches anything.
  ------------------------------------------------------------------------

  function "?=" (l, r : std_ulogic) return std_ulogic is
  begin
    if l = '-' or r = '-' then
      return '1';
    elsif l = 'U' or r = 'U' then
      return 'U';
    elsif to_x01z_table(l) = 'X' or to_x01z_table(r) = 'X' then
      return 'X';
    elsif to_x01z_table(l) = to_x01z_table(r) then
      return '1';
    else
      return '0';
    end if;
  end function "?=";

  function "?/=" (l, r : std_ulogic) return std_ulogic is
  begin
    return not_table(l ?= r);
  end function "?/=";

  function "?<" (l, r : std_ulogic) return std_ulogic is
  begin
    if l = 'U' or r = 'U' then
      return 'U';
    elsif to_x01_table(l) = 'X' or to_x01_table(r) = 'X' then
      return 'X';
    elsif to_x01_table(l) = '0' and to_x01_table(r) = '1' then
      return '1';
    else
      return '0';
    end if;
  end function "?<";

  function "?<=" (l, r : std_ulogic) return std_ulogic is
  begin
    return (l ?< r) or (l ?= r);
  end function "?<=";

  function "?>" (l, r : std_ulogic) return std_ulogic is
  begin
    return not_table(l ?<= r);
  end function "?>";

  function "?>=" (l, r : std_ulogic) return std_ulogic is
  begin
    return not_table(l ?< r);
  end function "?>=";

  function "?=" (l, r : std_ulogic_vector) return std_ulogic is
    alias lv : std_ulogic_vector (1 to l'length) is l;
    alias rv : std_ulogic_vector (1 to r'length) is r;
    variable result : std_ulogic := '1';
  begin
    assert l'length = r'length
      report "std_logic_1164.""?="": operands of different lengths"
      severity failure;
    for i in lv'range loop
      result := and_table(result, lv(i) ?= rv(i));
    end loop;
    return result;
  end function "?=";

  function "?/=" (l, r : std_ulogic_vector) return std_ulogic is
  begin
    return not_table(l ?= r);
  end function "?/=";

  function "??" (l : std_ulogic) return boolean is
  begin
    return l = '1' or l = 'H';
  end function "??";

  ------------------------------------------------------------------------
  -- Conversions.
  ------------------------------------------------------------------------

  function to_bit (s : std_ulogic; xmap : bit := '0') return bit is
  begin
    case s is
      when '0' | 'L' => return '0';
      when '1' | 'H' => return '1';
      when others    => return xmap;
    end case;
  end function to_bit;

  function to_bitvector (s : std_ulogic_vector; xmap : bit := '0') return bit_vector is
    alias sv : std_ulogic_vector (1 to s'length) is s;
    variable result : bit_vector (1 to s'length);
  begin
    for i in result'range loop
      result(i) := to_bit(sv(i), xmap);
    end loop;
    return result;
  end function to_bitvector;

  function to_stdulogic (b : bit) return std_ulogic is
  begin
    if b = '0' then
      return '0';
    else
      return '1';
    end if;
  end function to_stdulogic;

  function to_stdlogicvector (b : bit_vector) return std_logic_vector is
    alias bv : bit_vector (1 to b'length) is b;
    variable result : std_logic_vector (1 to b'length);
  begin
    for i in result'range loop
      result(i) := to_stdulogic(bv(i));
    end loop;
    return result;
  end function to_stdlogicvector;

  function to_stdulogicvector (b : bit_vector) return std_ulogic_vector is
    alias bv : bit_vector (1 to b'length) is b;
    variable result : std_ulogic_vector (1 to b'length);
  begin
    for i in result'range loop
      result(i) := to_stdulogic(bv(i));
    end loop;
    return result;
  end function to_stdulogicvector;

  function to_bit_vector (s : std_ulogic_vector; xmap : bit := '0') return bit_vector is
  begin
    return to_bitvector(s, xmap);
  end function to_bit_vector;

  function to_std_logic_vector (b : bit_vector) return std_logic_vector is
  begin
    return to_stdlogicvector(b);
  end function to_std_logic_vector;

  function to_std_ulogic_vector (b : bit_vector) return std_ulogic_vector is
  begin
    return to_stdulogicvector(b);
  end function to_std_ulogic_vector;

  ------------------------------------------------------------------------
  -- Strength strippers.
  ------------------------------------------------------------------------

  function to_x01 (s : std_ulogic_vector) return std_ulogic_vector is
    alias sv : std_ulogic_vector (1 to s'length) is s;
    variable result : std_ulogic_vector (1 to s'length);
  begin
    for i in result'range loop
      result(i) := to_x01_table(sv(i));
    end loop;
    return result;
  end function to_x01;

  function to_x01 (s : std_ulogic) return x01 is
  begin
    return to_x01_table(s);
  end function to_x01;

  function to_x01 (b : bit_vector) return std_ulogic_vector is
  begin
    return to_stdulogicvector(b);
  end function to_x01;

  function to_x01 (b : bit) return x01 is
  begin
    return to_stdulogic(b);
  end function to_x01;

  function to_x01z (s : std_ulogic_vector) return std_ulogic_vector is
    alias sv : std_ulogic_vector (1 to s'length) is s;
    variable result : std_ulogic_vector (1 to s'length);
  begin
    for i in result'range loop
      result(i) := to_x01z_table(sv(i));
    end loop;
    return result;
  end function to_x01z;

  function to_x01z (s : std_ulogic) return x01z is
  begin
    return to_x01z_table(s);
  end function to_x01z;

  function to_x01z (b : bit_vector) return std_ulogic_vector is
  begin
    return to_stdulogicvector(b);
  end function to_x01z;

  function to_x01z (b : bit) return x01z is
  begin
    return to_stdulogic(b);
  end function to_x01z;

  function to_ux01 (s : std_ulogic_vector) return std_ulogic_vector is
    alias sv : std_ulogic_vector (1 to s'length) is s;
    variable result : std_ulogic_vector (1 to s'length);
  begin
    for i in result'range loop
      result(i) := to_ux01_table(sv(i));
    end loop;
    return result;
  end function to_ux01;

  function to_ux01 (s : std_ulogic) return ux01 is
  begin
    return to_ux01_table(s);
  end function to_ux01;

  function to_ux01 (b : bit_vector) return std_ulogic_vector is
  begin
    return to_stdulogicvector(b);
  end function to_ux01;

  function to_ux01 (b : bit) return ux01 is
  begin
    return to_stdulogic(b);
  end function to_ux01;

  function to_01 (s : std_ulogic_vector; xmap : std_ulogic := '0') return std_ulogic_vector is
    alias sv : std_ulogic_vector (1 to s'length) is s;
    variable result : std_ulogic_vector (1 to s'length);
  begin
    for i in result'range loop
      result(i) := to_01(sv(i), xmap);
    end loop;
    return result;
  end function to_01;

  function to_01 (s : std_ulogic; xmap : std_ulogic := '0') return std_ulogic is
  begin
    case s is
      when '0' | 'L' => return '0';
      when '1' | 'H' => return '1';
      when others    => return xmap;
    end case;
  end function to_01;

  function is_x (s : std_ulogic_vector) return boolean is
  begin
    for i in s'range loop
      if is_x(s(i)) then
        return true;
      end if;
    end loop;
    return false;
  end function is_x;

  function is_x (s : std_ulogic) return boolean is
  begin
    case s is
      when '0' | '1' | 'L' | 'H' => return false;
      when others                => return true;
    end case;
  end function is_x;

  ------------------------------------------------------------------------
  -- Edge detection. The event must take the signal to a forcing or weak
  -- high (low) from a value that was not already high (low).
  ------------------------------------------------------------------------

  function rising_edge (signal s : std_ulogic) return boolean is
  begin
    return s'event
       and to_x01_table(s) = '1'
       and to_x01_table(s'last_value) = '0';
  end function rising_edge;

  function falling_edge (signal s : std_ulogic) return boolean is
  begin
    return s'event
       and to_x01_table(s) = '0'
       and to_x01_table(s'last_value) = '1';
  end function falling_edge;

  ------------------------------------------------------------------------
  -- String rendering (VHDL-2008).
  ------------------------------------------------------------------------

  function to_string (value : std_ulogic) return string is
    variable result : string (1 to 1);
  begin
    case value is
      when 'U' => result := "U";
      when 'X' => result := "X";
      when '0' => result := "0";
      when '1' => result := "1";
      when 'Z' => result := "Z";
      when 'W' => result := "W";
      when 'L' => result := "L";
      when 'H' => result := "H";
      when '-' => result := "-";
    end case;
    return result;
  end function to_string;

  function to_string (value : std_ulogic_vector) return string is
    alias v : std_ulogic_vector (1 to value'length) is value;
    variable result : string (1 to value'length);
  begin
    for i in result'range loop
      result(i to i) := to_string(v(i));
    end loop;
    return result;
  end function to_string;

  function to_bstring (value : std_ulogic_vector) return string is
  begin
    return to_string(value);
  end function to_bstring;

  -- Renders `value` in groups of `bits_per_digit`, left-padded with '0',
  -- with a group containing any non-0/1 element rendered as 'X'.
  function to_radix_string (value : std_ulogic_vector; bits_per_digit : positive)
    return string
  is
    constant pad    : natural := (bits_per_digit - value'length mod bits_per_digit)
                                 mod bits_per_digit;
    constant padded : std_ulogic_vector (1 to value'length + pad) :=
      (1 to pad => '0') & to_x01(value);
    variable result : string (1 to padded'length / bits_per_digit);
    variable digit  : natural;
    variable known  : boolean;
    variable base   : natural;
  begin
    for d in result'range loop
      digit := 0;
      known := true;
      base  := (d - 1) * bits_per_digit;
      for b in 1 to bits_per_digit loop
        case padded(base + b) is
          when '0'    => digit := digit * 2;
          when '1'    => digit := digit * 2 + 1;
          when others => known := false;
        end case;
      end loop;
      if known then
        result(d to d) := hex_digits(digit + 1 to digit + 1);
      else
        result(d to d) := "X";
      end if;
    end loop;
    return result;
  end function to_radix_string;

  function to_ostring (value : std_ulogic_vector) return string is
  begin
    return to_radix_string(value, 3);
  end function to_ostring;

  function to_hstring (value : std_ulogic_vector) return string is
  begin
    return to_radix_string(value, 4);
  end function to_hstring;

end package body std_logic_1164;
