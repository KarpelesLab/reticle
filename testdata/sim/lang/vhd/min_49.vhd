library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  constant G : string := "hello";
begin
  process
  begin
    if G'length = 5 then report "string length ok"; else report "string length bad"; end if;
    wait;
  end process;
end architecture;
