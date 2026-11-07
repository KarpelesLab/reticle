library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal a : std_logic_vector(7 downto 0) := x"C3";
  procedure show(r : std_logic_vector(7 downto 0)) is
  begin
    for k in 7 downto 0 loop
      if r(k) = '1' then report "1"; elsif r(k) = '0' then report "0"; else report "other"; end if;
    end loop;
  end procedure;
begin
  process
    variable r : std_logic_vector(7 downto 0);
    variable bv : bit_vector(7 downto 0) := "10110001";
    variable bw : bit_vector(7 downto 0);
  begin
    report "a sll 2 (expect 00001100)";
    r := a sll 2;
    show(r);
    report "a srl 3 (expect 00011000)";
    r := a srl 3;
    show(r);
    report "bv sra 2 (expect 11101100)";
    bw := bv sra 2;
    show(to_stdlogicvector(bw));
    wait;
  end process;
end architecture;
