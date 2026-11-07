library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
  begin
    report "a" & "b" & "c";
    wait;
  end process;
end architecture;
