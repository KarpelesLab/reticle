library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity tb is end entity;
architecture sim of tb is
  signal clk : std_logic := '0';
begin
  process
  begin
    clk <= '1' after 5 ns;
    wait until rising_edge(clk);
    report "rose";
    wait;
  end process;
end architecture;
