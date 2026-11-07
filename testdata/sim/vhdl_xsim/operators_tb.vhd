-- Operators whose operands or answers were wrong, held to what Vivado
-- 2026.1's xsim printed for this file (operators_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is end entity;

architecture sim of tb is
  function fv(a, b : std_logic_vector(7 downto 0)) return std_logic_vector is
    variable v : std_logic_vector(7 downto 0);
  begin
    v := a xor b;
    return v;
  end function;
  function fn(a : std_logic_vector(7 downto 0)) return std_logic_vector is
  begin
    return not a;
  end function;
  function fb(a, b : std_logic) return std_logic is
  begin
    return (a xor b) and not (a and b);
  end function;
  function decq(v : std_logic_vector(2 downto 0)) return integer is
  begin
    case? v is
      when "000" => return 0;
      when "01-" => return 2;
      when "1--" => return 4;
      when others => return 6;
    end case?;
  end function;
  signal prio : std_logic_vector(3 downto 0) := "0110";
  signal y : std_logic_vector(1 downto 0);
  signal x : std_logic := 'X';
begin
  with prio select?
    y <= "11" when "1---",
         "10" when "01--",
         "01" when "001-",
         "00" when others;

  process
    variable u : unsigned(7 downto 0) := x"A6";
    variable a4 : std_logic_vector(3 downto 0) := "1100";
    variable v : std_logic_vector(7 downto 0) := x"C3";
    variable r : std_logic_vector(7 downto 0);
    variable bv : bit_vector(7 downto 0) := "10110001";
    variable bw : bit_vector(7 downto 0);
    variable n : integer := 3;
  begin
    -- Literals whose length is their own, not the context's.
    if u = x"A6" and u = "10100110" and signed(u) = "10100110" then report "literal ="; end if;
    if (a4 & a4) = x"CC" then report "concat ="; end if;
    r := v(5 downto 0) & "00";
    report "slice & literal " & to_string(r);
    r := v(5 downto 0) & '0' & '1';
    report "slice & chars " & to_string(r);
    -- std_logic functions folded with constant arguments.
    report "fv " & to_hstring(fv(x"0F", x"05")) & " fn " & to_hstring(fn(x"0F"))
      & " fb " & std_logic'image(fb('1', '0')) & std_logic'image(fb('1', '1'));
    -- `??` and `?=`, and `'-'`.
    if ?? 'X' then report "?? X true"; else report "?? X false"; end if;
    if ?? x then report "?? sig X true"; else report "?? sig X false"; end if;
    report "0 ?= - is " & std_logic'image('0' ?= '-');
    report "1010 ?= 1-1- is " & std_logic'image(std_logic_vector'("1010") ?= "1-1-");
    report "1010 ?/= 0--- is " & std_logic'image(std_logic_vector'("1010") ?/= "0---");
    report "X ?= X is " & std_logic'image(x ?= 'X');
    report "decq " & integer'image(decq("000")) & integer'image(decq("011"))
      & integer'image(decq("101")) & integer'image(decq("001"));
    wait for 1 ns;
    report "select? gives " & to_string(y);
    -- Shifts and rotates of std_logic_vector and bit_vector.
    report "sll " & to_string(v sll 2) & " srl " & to_string(v srl 3)
      & " rol " & to_string(v rol 1) & " ror " & to_string(v ror n);
    report "sll -2 " & to_string(v sll -2) & " rol by var " & to_string(v rol n);
    bw := bv sra 2;
    report "sra " & to_string(bw) & " sla " & to_string(bv sla 3)
      & " srl " & to_string(bv srl n) & " ror " & to_string(bv ror 9);
    wait;
  end process;
end architecture;
