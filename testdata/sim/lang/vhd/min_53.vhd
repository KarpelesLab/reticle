library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal w : std_logic_vector(7 downto 0) := x"80";
  alias rev : std_logic_vector(0 to 7) is w(7 downto 0);
begin
  process
  begin
    if rev(0) = '1' then report "rev(0) is w(7) ok"; else report "rev(0) is not w(7)"; end if;
    wait;
  end process;
end architecture;
