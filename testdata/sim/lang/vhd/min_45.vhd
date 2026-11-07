library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal cnt : integer := 3;
  impure function read_cnt return integer is
  begin
    return cnt;
  end function;
begin
  process
  begin
    wait for 1 ns;
    if read_cnt = 3 then report "impure ok"; else report "impure bad"; end if;
    wait;
  end process;
end architecture;
