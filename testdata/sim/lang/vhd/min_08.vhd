library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
  begin
    assert false;
    assert false severity warning;
    assert 1 = 0 severity note;
    wait;
  end process;
end architecture;
