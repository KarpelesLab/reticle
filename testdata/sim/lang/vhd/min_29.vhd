library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  function classify(n : integer) return integer is
  begin
    case n is
      when integer'low to -1 => return 1;
      when others => return 2;
    end case;
  end function;
begin
  process
  begin
    if classify(-5) = 1 then report "big range choice ok"; else report "big range choice bad"; end if;
    wait;
  end process;
end architecture;
