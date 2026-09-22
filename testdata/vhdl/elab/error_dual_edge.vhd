-- V0701: a process that tests both edges of a clock has no shape in the
-- IR's sequential process, and is named as unsupported.
library ieee;
use ieee.std_logic_1164.all;

entity error_dual_edge is
  port (
    clk  : in  std_logic;
    a, b : in  std_logic;
    q    : out std_logic
  );
end entity;

architecture rtl of error_dual_edge is
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= a;
    elsif falling_edge(clk) then
      q <= b;
    end if;
  end process;
end architecture;
