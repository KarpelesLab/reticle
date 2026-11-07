library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
begin
  process
  begin
    report character'image('A');
    report "upper " & character'image('Q');
    if character'image('A') = "'A'" then report "char image ok"; else report "char image lowercased"; end if;
    wait;
  end process;
end architecture;
