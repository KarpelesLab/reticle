library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  function decq(v : std_logic_vector(2 downto 0)) return integer is
  begin
    case? v is
      when "000" => return 0;
      when "01-" => return 2;
      when "1--" => return 4;
      when others => return 6;
    end case?;
  end function;
begin
  process
  begin
    if decq("000") = 0 then report "000 ok"; else report "000 bad"; end if;
    if decq("011") = 2 then report "011 ok"; else report "011 bad"; end if;
    if decq("101") = 4 then report "101 ok"; else report "101 bad"; end if;
    if decq("001") = 6 then report "001 ok"; else report "001 bad"; end if;
    wait;
  end process;
end architecture;
