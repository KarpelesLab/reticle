-- A counter whose output is gray coded, so at most one bit changes per
-- clock. Written in VHDL to prove a project can depend on IP in either
-- language.
library ieee;
use ieee.std_logic_1164.all;

entity gray_counter is
  port (
    clk  : in  std_logic;
    rst  : in  std_logic;
    gray : out std_logic_vector(3 downto 0)
  );
end entity;

architecture rtl of gray_counter is
  signal count : std_logic_vector(3 downto 0);
begin
  process (clk, rst)
  begin
    if rst = '1' then
      count <= "0000";
    elsif rising_edge(clk) then
      count(0) <= not count(0);
      count(1) <= count(1) xor count(0);
      count(2) <= count(2) xor (count(1) and count(0));
      count(3) <= count(3) xor (count(2) and count(1) and count(0));
    end if;
  end process;

  gray(3) <= count(3);
  gray(2) <= count(3) xor count(2);
  gray(1) <= count(2) xor count(1);
  gray(0) <= count(1) xor count(0);
end architecture;
