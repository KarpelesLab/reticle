library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  type small_t is range 0 to 255;
  subtype byte_t is std_logic_vector(7 downto 0);
  type my_vec is array (natural range <>) of std_logic;
begin
  process
    variable i : integer;
    variable sm : small_t;
    variable b : byte_t;
    variable u : unsigned(7 downto 0);
    variable s : signed(7 downto 0);
    variable mv : my_vec(7 downto 0);
    variable bv : bit_vector(3 downto 0);
    variable bt : bit;
    variable sl : std_logic;
  begin
    sm := 200;
    i := integer(sm);
    if i = 200 then report "int(small) ok"; else report "int(small) bad"; end if;
    sm := small_t(i / 2);
    if sm = 100 then report "small(int) ok"; else report "small(int) bad"; end if;
    b := x"F6";
    u := unsigned(b);
    s := signed(b);
    if to_integer(u) = 246 and to_integer(s) = -10 then report "slv to u/s ok"; else report "slv to u/s bad"; end if;
    b := std_logic_vector(to_signed(-128, 8));
    if b = x"80" then report "signed to slv ok"; else report "signed to slv bad"; end if;
    mv := my_vec(b);
    if mv(7) = '1' and mv(0) = '0' then report "closely related array ok"; else report "closely related array bad"; end if;
    b := std_logic_vector(mv);
    if b = x"80" then report "back to slv ok"; else report "back to slv bad"; end if;
    bv := to_bitvector(std_logic_vector'("1H0L"));
    if bv = "1100" then report "to_bitvector ok"; else report "to_bitvector bad"; end if;
    b(3 downto 0) := to_stdlogicvector(bv);
    if b(3 downto 0) = "1100" then report "to_stdlogicvector ok"; else report "to_stdlogicvector bad"; end if;
    bt := to_bit('H');
    sl := to_stdulogic(bt);
    if bt = '1' and sl = '1' then report "to_bit ok"; else report "to_bit bad"; end if;
    i := to_integer(unsigned'("1010"));
    if i = 10 then report "qualified unsigned ok"; else report "qualified unsigned bad"; end if;
    i := to_integer(signed'("1010"));
    if i = -6 then report "qualified signed ok"; else report "qualified signed bad"; end if;
    if byte_t'(others => '1') = x"FF" then report "qualified aggregate ok"; else report "qualified aggregate bad"; end if;
    i := integer'(5) + natural'(3);
    if i = 8 then report "qualified scalar ok"; else report "qualified scalar bad"; end if;
    i := character'pos('a') - character'pos('A');
    if i = 32 then report "char pos ok"; else report "char pos bad"; end if;
    if boolean'pos(true) = 1 then report "bool pos ok"; else report "bool pos bad"; end if;
    sl := std_logic'val(3);
    if sl = '1' then report "sl val ok"; else report "sl val bad"; end if;
    i := to_integer(unsigned(std_logic_vector'(x"0" & "1111")));
    if i = 15 then report "concat qualified ok"; else report "concat qualified bad"; end if;
    i := integer(real(7) / 2.0);
    if i = 4 then report "real round ok"; else report "real round bad"; end if;
    wait;
  end process;
end architecture;
