library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
begin
  process
    variable u : unsigned(7 downto 0) := "10110001";
    variable s : signed(7 downto 0) := "10110001";
    variable bv : bit_vector(7 downto 0) := "10110001";
  begin
    report "shl u 3=" & to_string(shift_left(u, 3));
    report "shr u 3=" & to_string(shift_right(u, 3));
    report "shl s 3=" & to_string(shift_left(s, 3));
    report "shr s 3=" & to_string(shift_right(s, 3));
    report "rotl u 3=" & to_string(rotate_left(u, 3));
    report "rotr u 3=" & to_string(rotate_right(u, 3));
    report "rotl s 9=" & to_string(rotate_left(s, 9));
    report "shr u 10=" & to_string(shift_right(u, 10));
    report "shr s 10=" & to_string(shift_right(s, 10));
    report "u sll 2=" & to_string(u sll 2);
    report "u srl 2=" & to_string(u srl 2);
    report "s sll 2=" & to_string(s sll 2);
    report "s srl 2=" & to_string(s srl 2);
    report "u rol 2=" & to_string(u rol 2);
    report "u ror 2=" & to_string(u ror 2);
    report "s sla 2=" & to_string(s sla 2);
    report "s sra 2=" & to_string(s sra 2);
    report "u sra 2=" & to_string(u sra 2);
    report "u sla 1=" & to_string(u sla 1);
    report "s sra -2=" & to_string(s sra -2);
    report "bv sll 2=" & to_string(bv sll 2);
    report "bv srl 2=" & to_string(bv srl 2);
    report "bv sla 2=" & to_string(bv sla 2);
    report "bv sra 2=" & to_string(bv sra 2);
    report "bv rol 3=" & to_string(bv rol 3);
    report "bv ror 3=" & to_string(bv ror 3);
    report "bv sra -1=" & to_string(bv sra -1);
    report "shl to_integer=" & integer'image(to_integer(shift_left(to_unsigned(3, 8), 4)));
    wait;
  end process;
end architecture;
