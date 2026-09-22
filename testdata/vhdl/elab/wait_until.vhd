-- `wait until rising_edge(clk)` as the first statement of a process makes
-- it a clocked process whose body is what follows.
library ieee;
use ieee.std_logic_1164.all;

entity wait_until is
  port (
    clk : in  std_logic;
    d   : in  std_logic_vector(3 downto 0);
    q   : out std_logic_vector(3 downto 0)
  );
end entity;

architecture rtl of wait_until is
begin
  process
  begin
    wait until rising_edge(clk);
    q <= d;
  end process;
end architecture;
