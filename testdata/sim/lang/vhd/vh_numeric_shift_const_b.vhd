library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb is
end entity;

architecture sim of tb is
  constant U0 : unsigned(7 downto 0) := "10110001";
  constant S0 : signed(7 downto 0) := "10110001";
begin
  process
    variable u : unsigned(7 downto 0) := U0;
    variable s : signed(7 downto 0) := S0;
    variable r : unsigned(7 downto 0);
    variable rs : signed(7 downto 0);
    variable bv : bit_vector(7 downto 0) := "10110001";
  begin
    r := shift_left(u, 3);
    if std_logic_vector(r) = "10001000" then report "shl u ok"; else report "shl u bad"; end if;
    r := shift_right(u, 3);
    if std_logic_vector(r) = "00010110" then report "shr u ok"; else report "shr u bad"; end if;
    rs := shift_right(s, 3);
    if std_logic_vector(rs) = "11110110" then report "shr s ok"; else report "shr s bad"; end if;
    r := rotate_left(u, 3);
    if std_logic_vector(r) = "10001101" then report "rotl ok"; else report "rotl bad"; end if;
    r := rotate_right(u, 3);
    if std_logic_vector(r) = "00110110" then report "rotr ok"; else report "rotr bad"; end if;
    rs := rotate_left(s, 9);
    if std_logic_vector(rs) = "01100011" then report "rotl9 ok"; else report "rotl9 bad"; end if;
    rs := shift_right(s, 10);
    if std_logic_vector(rs) = "11111111" then report "shr s 10 ok"; else report "shr s 10 bad"; end if;
    r := u srl 2;
    if std_logic_vector(r) = "00101100" then report "srl ok"; else report "srl bad"; end if;
    rs := s sra 2;
    if std_logic_vector(rs) = "11101100" then report "sra ok"; else report "sra bad"; end if;
    rs := s sla 2;
    if std_logic_vector(rs) = "11000100" then report "sla ok"; else report "sla bad"; end if;
    r := u ror 2;
    if std_logic_vector(r) = "01101100" then report "ror ok"; else report "ror bad"; end if;
    if to_integer(shift_left(to_unsigned(3, 8), 4)) = 48 then report "shl int ok"; else report "shl int bad"; end if;
    if to_integer(shift_right(to_signed(-64, 8), 3)) = -8 then report "shr neg ok"; else report "shr neg bad"; end if;
    bv := bv sra 2;
    if bv = "11101100" then report "bv sra ok"; else report "bv sra bad"; end if;
    bv := bv sla 1;
    if bv = "11001111" then report "bv sla ok"; else report "bv sla bad"; end if;
    wait;
  end process;
end architecture;
