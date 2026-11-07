library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal a : std_logic_vector(7 downto 0) := x"00";
  procedure drive(signal s : out std_logic_vector; v : in std_logic_vector) is
  begin
    s <= v;
  end procedure;
begin
  process
  begin
    drive(a, x"5A");
    wait for 1 ns;
    if a = x"5A" then report "unconstrained signal param ok"; else report "unconstrained signal param bad"; end if;
    wait;
  end process;
end architecture;
