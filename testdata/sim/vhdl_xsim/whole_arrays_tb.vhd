-- Arrays of wide elements read and assigned whole or by slices, and string
-- objects of a fixed length, held to what Vivado 2026.1's xsim printed for
-- this file (whole_arrays_tb.xsim).
library ieee;
use ieee.std_logic_1164.all;

entity tb is end entity;

architecture sim of tb is
  type int_arr is array (0 to 3) of integer;
  type word_arr is array (0 to 3) of std_logic_vector(7 downto 0);
  constant GREETING : string := "hello";
  signal w : word_arr := (x"11", x"22", x"33", x"44");
begin
  process
    variable ia, ib : int_arr;
    variable s : string(1 to 5);
    variable n : integer := 42;
  begin
    ia := (others => 7);
    report "ia " & integer'image(ia(0)) & integer'image(ia(3));
    ib := ia;
    ib(2) := 5;
    if ia /= ib then report "copies differ"; end if;
    ib(2) := 7;
    if ia = ib then report "copies equal"; end if;
    ia := (1, 2, 3, 4);
    ib(1 to 2) := ia(2 to 3);
    report "ib " & integer'image(ib(0)) & integer'image(ib(1)) & integer'image(ib(2))
      & integer'image(ib(3));
    w(0 to 1) <= w(2 to 3);
    wait for 1 ns;
    report "w " & to_hstring(w(0)) & to_hstring(w(1)) & to_hstring(w(2)) & to_hstring(w(3));
    s := GREETING;
    report "s=" & s & " s(2)=" & s(2);
    s(1) := 'J';
    report "modified " & s;
    if s = "Jello" then report "string compares"; end if;
    wait;
  end process;
end architecture;
