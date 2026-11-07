library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  type word_arr is array (0 to 3) of std_logic_vector(7 downto 0);
  signal w : word_arr := (x"11", x"22", x"33", x"44");
begin
  process
  begin
    w(0 to 1) <= w(2 to 3);
    wait for 1 ns;
    if w(0) = x"33" and w(1) = x"44" then report "arr slice assign ok"; else report "arr slice assign bad"; end if;
    wait;
  end process;
end architecture;
