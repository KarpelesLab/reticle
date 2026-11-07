library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal prio : std_logic_vector(3 downto 0) := "0110";
  signal y : std_logic_vector(1 downto 0);
begin
  with prio select?
    y <= "11" when "1---",
         "10" when "01--",
         "01" when "001-",
         "00" when others;
  process
  begin
    wait for 1 ns;
    if y = "10" then report "select? 0110 ok"; elsif y = "00" then report "select? fell to others"; else report "select? other"; end if;
    wait;
  end process;
end architecture;
