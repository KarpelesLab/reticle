library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  function popcount(v : std_logic_vector) return natural is
    variable c : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then c := c + 1; end if;
    end loop;
    return c;
  end function;
  signal s : std_logic_vector(7 downto 0) := x"F1";
begin
  process
  begin
    if popcount(s) = 5 then report "unconstrained param ok"; else report "unconstrained param bad"; end if;
    wait;
  end process;
end architecture;
