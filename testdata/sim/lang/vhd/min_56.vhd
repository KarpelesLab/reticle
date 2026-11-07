library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal a : std_logic := '0';
  signal d : std_logic := '-';
begin
  process
  begin
    if (a ?= '-') = '1' then report "0 ?= - is 1 ok"; else report "0 ?= - not 1"; end if;
    if (a ?= d) = '1' then report "0 ?= sig - is 1 ok"; else report "0 ?= sig - not 1"; end if;
    wait;
  end process;
end architecture;
